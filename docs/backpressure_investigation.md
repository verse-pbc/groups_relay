# Backpressure Handling Guide for `websocket_builder`

## 1. High-Level Overview

`websocket_builder` enforces backpressure through a **bounded** `tokio::sync::mpsc` queue that sits between your application logic (middleware, tasks, etc.) and the WebSocket writer.  All outbound messages must pass through this queue.

* **Producers** call `MessageSender::send()` (a thin wrapper around `mpsc::Sender::try_send`).
* **Consumers** (the connection's main `select!` loop) dequeue messages and write them to the socket.
* When the queue is **full**, `try_send` returns `Err(TrySendError::Full)` **immediately** – _it never waits_.  This error is the library's backpressure signal.

This design guarantees that _nothing in the inbound path ever awaits on the socket writer_, eliminating an entire class of dead-lock scenarios common in naïve WebSocket handlers.

---

## 2. Key Components

| Component                                   | Responsibility                                                      |
|---------------------------------------------|----------------------------------------------------------------------|
| `MessageSender<O>`                          | Wraps `mpsc::Sender<(O, usize)>` and exposes a non-blocking `send()`. |
| Bounded `mpsc` queue                        | Buffers outbound messages; its **capacity** is configured per connection. |
| WebSocket main loop (`tokio::select!`)      | Dequeues (`recv().await`) and writes (`socket.send().await`).         |
| Application / middleware / background tasks | Produce outbound messages by calling `MessageSender::send(...)`.      |

---

## 3. Data-Flow Diagram

```text
[middleware / tasks] ──► try_send() ──► BOUNDED MPSC QUEUE ──► select! loop ──► outbound middleware chain ──► socket.send()
```

1. **Produce** – Your code enqueues `(message, origin_middleware_index)` via `try_send()`.
2. **Buffer** – The bounded queue absorbs short bursts; when full, it pushes back (error).
3. **Drain** – The main loop drains the queue whenever the socket is writable (its **branch B**).

---

## 4. Why Non-Blocking `try_send`?

1. **Avoid Circular Waits**
   In the classic dead-lock: inbound handler → `socket.send().await` → waiting for writer task → blocked because inbound handler still holds the executor.  By **never awaiting**, the inbound path completes and frees the executor so branch B can run.
2. **Backpressure as a Fast Fail**
   The caller **instantly** learns that the consumer cannot keep up and can react (drop, retry, close…).
3. **Predictable Latency**
   Long `await`s inside latency-sensitive middleware are avoided.

---

## 5. Interpreting the Backpressure Signal

`MessageSender::send()` returns `Result<(), TrySendError<O>>`.

* **`Ok(())`** – Message accepted.
* **`Err(TrySendError::Full(_msg))`** – Queue full → the consumer is slower than producers.
* **`Err(TrySendError::Closed(_msg))`** – Connection is already closed.

### Recommended Strategies

| Use-case                                        | Reaction to `Full`                                           |
|-------------------------------------------------|--------------------------------------------------------------|
| Non-critical updates (e.g. presence, pings)     | **Drop the message** silently.                               |
| Finite retry acceptable                          | **Retry with back-off** (e.g. exponential delay).            |
| Delivery must be guaranteed (rare in WebSockets) | **Close & reopen** the connection with upstream notice.      |

_Choose the lightest strategy that matches your reliability requirements – blocking the producer defeats the purpose._

---

## 6. Sizing the Queue

`MessageSender::with_capacity(capacity)` lets you tune the buffer per connection.  Consider:

* **Throughput vs. Memory** – Bigger buffer masks bursts but consumes more RAM.
* **Latency Budgets** – Larger capacity postpones the backpressure signal; pick a size that surfaces pressure _before_ SLAs are breached.
* **Worst-Case Burst** – Estimate the largest burst your application can produce in one event loop tick.

> **Rule of Thumb:** Start with 64–256 messages; instrument and adjust.

---

## 7. Dead-Lock Avoidance Explained

```text
select! {
    // branch A – inbound traffic
    Some(frame) = socket.next() => {
        run_inbound_middleware(frame);      // QUICK → never await on writer
    }
    // branch B – outbound traffic
    Some(out) = queue.recv() => {
        run_outbound_middleware_chain(&out);
        socket.send(out).await?;
    }
}
```

* **Invariant:** Code executed inside branch A **must not await on** the writer; instead, enqueue via `MessageSender`.
* Because branch A finishes quickly, the executor can poll the `select!` again and let branch B flush the queue.
* Thus the classic _"read handler awaits write which needs read handler to return"_ cycle cannot happen.

---

## 8. Best-Practice Checklist for Middleware Authors

✔️ Use `ctx.send_message(msg)` – never call `socket.send()` directly.
✔️ Treat `TrySendError::Full` as a _normal_ control-flow event.
✔️ Keep inbound work lightweight; spawn tasks for heavy DB / CPU work.
✔️ Monitor queue length (`Sender::capacity()` & custom metrics).
✔️ Prefer dropping or debouncing repetitive updates over blocking.

---

## 9. Alternative Designs

For a deeper comparison (dedicated writer task, blocking `send().await`, priority queues, …) see [`docs/backpressure_alternatives.md`](mdc:backpressure_alternatives.md).

---

## 10. FAQ

**Q: Can I make `send()` await instead of failing?**
A: You _can_ wrap it in a manual retry loop with `tokio::time::sleep`, but _blocking inside the inbound path_ re-introduces the dead-lock hazard. Spawn a new task if you must await.

**Q: How do I detect persistent backpressure?**
A: Instrument the queue length (difference between capacity & `Sender::capacity()`) or count consecutive `Full` errors. When above threshold, disconnect or apply back-off.

**Q: The queue is full right after connection. What gives?**
A: Your app produced a burst before the main loop had a chance to start draining; either increase capacity or defer heavy startup messages.

---

## 11. Minimal Example

```rust
async fn my_middleware(ctx: &mut InboundContext<'_>, msg: RelayMessage) -> Result<()> {
    // Transform inbound → outbound
    let outbound = RelayMessage::Echo { data: msg.data.clone() };

    // Non-blocking send – may error if queue is full
    if let Err(err) = ctx.send_message(outbound) {
        match err {
            TrySendError::Full(_) => {
                tracing::warn!("queue full – dropping echo reply");
            }
            TrySendError::Closed(_) => return Err(anyhow!("connection closed")),
        }
    }
    Ok(())
}
```