# Backpressure ‑ Alternative Designs & Future Refactor Ideas

This document collects **possible refactors** for the WebSocket send/receive architecture.  It is *exploratory only* – nothing here is implemented.

## 1. Split Reader / Writer Tasks (`socket.split()`)

````rust
let (mut ws_tx, mut ws_rx) = socket.split();

// Task R – read loop
let read_task = tokio::spawn(async move {
    while let Some(frame) = ws_rx.next().await { /* inbound middleware */ }
});

// Task W – write loop fed by an mpsc::Receiver
let write_task = tokio::spawn(async move {
    while let Some((msg, idx)) = outbound_rx.recv().await {
        /* outbound middleware starting at idx */
        ws_tx.send(Message::Text(msg)).await?;
    }
});
````

Pros
- Total isolation: slow inbound work never blocks writes.
- Back-pressure handled naturally via `Sender::send().await` (producer awaits space).

Cons
- **Extra task per connection** (R & W) instead of one.
- Shared state (`TapState`) now mutated from two tasks → need `Mutex`/`RwLock` or channel hand-off.
- Graceful shutdown more complex (coordinate both tasks & connection timeout).
- Still need to remember `middleware_index` inside queued message.

### Preserving Sequential State Semantics with an Actor Task

A pragmatic way to keep today's **strict in-order state mutations** while still gaining the
benefits of a split reader/writer is to introduce a *single owner* task – an **actor** – that owns
`TapState` and processes **commands** sent from the reader and writer loops:

````rust
// Each connection spawns exactly three tasks
// 1. reader  2. writer  3. state-actor (owns TapState)
let (state_cmd_tx, mut state_cmd_rx) = tokio::sync::mpsc::unbounded_channel::<StateCmd>();

// -- Task A: Reader ----------------------------------------------------------
let reader_state_tx = state_cmd_tx.clone();
let read_task = tokio::spawn(async move {
    while let Some(frame) = ws_rx.next().await {
        // ... inbound middleware ...
        // mutate shared state by *sending* a command, never by direct access
        let _ = reader_state_tx.send(StateCmd::Mutate(|s| s.last_seen = Instant::now()));
    }
});

// -- Task B: Writer ----------------------------------------------------------
let writer_state_tx = state_cmd_tx.clone();
let write_task = tokio::spawn(async move {
    while let Some((msg, idx)) = outbound_rx.recv().await {
        // need read-only snapshot? ask the actor
        let (resp_tx, resp_rx) = oneshot::channel();
        let _ = writer_state_tx.send(StateCmd::GetSnapshot(resp_tx));
        let snapshot = resp_rx.await?;
        // ... outbound middleware uses snapshot ...
        ws_tx.send(Message::Text(msg)).await?;
    }
});

// -- Task C: Actor -----------------------------------------------------------
let actor_task = tokio::spawn(async move {
    let mut state = TapState::default();
    while let Some(cmd) = state_cmd_rx.recv().await {
        match cmd {
            StateCmd::Mutate(f) => f(&mut state),
            StateCmd::GetSnapshot(resp_tx) => {
                let _ = resp_tx.send(state.clone()); // if TapState: Clone
            }
        }
    }
});
````

*   **Sequential guarantee** – The actor processes commands **one at a time** in the order
    received, so all mutations happen after each other exactly like the current single-loop model.
*   **No `Mutex` contention** – Only the actor owns the data; other tasks communicate through
    channels.
*   **Extensible** – You can add more command variants (metrics, shutdown, etc.) without leaking
    locks into middleware code.

With this in place the reader and writer remain fully asynchronous, yet your middleware stays
free of locks *and* retains deterministic state evolution.

When It Might Be Worth It
- Profiling shows outbound latency spikes caused by heavy inbound work.
- Memory cost of two tasks per connection is acceptable.

---

## 2. Blocking `Sender::send().await` Instead of `try_send()`

Make producers await for space rather than error.

Pros
- Simpler producer logic (no error path).
- Natural back-pressure – producers slow down instead of dropping.

Cons
- Risk of **re-introducing the dead-lock** if a producer is inside branch A (reader) because it would await indefinitely while holding the loop.
- Could stall high-priority messages coming from subscription task.

### Verdict
Not acceptable unless the reader/writer are split (option 1).

---

## 3. Priority Channels / Multiple Queues

Maintain two MPSC channels: High & Normal.  The select! loop drains High first.

Pros
- Prevents chatty spam from starving control messages (e.g., CLOSE, PING).

Cons
- More complexity, scheduler fairness still manual.
- Risk of unbounded high-prio DOS.

---

## 4. Off-loading Heavy Work to Background Tasks

Keep the current single-loop + queue design but move DB / crypto heavy ops to background tasks so branch A returns quickly.

Pros
- Avoids architectural change; reduces loop blocking window.

Cons
- Still some blocking window (task spawning cost).
- More boilerplate in middleware.

---

## 5. Unlimited/Unbounded Channel (Don't!)

Allows infinite buffering.

Cons
- Memory blow-up under high fan-out (many broadcasts, slow client).
- Hides back-pressure – producer never sees failure.

---

## Recommendation

Remain with **bounded MPSC + non-blocking `try_send`** for now.  Monitor metrics:

* queue full error counts
* per-connection queue occupancy
* outbound latency vs inbound processing time

If outbound latency becomes an issue and error rate is high even after tuning buffer sizes, revisit **Option 1 (split writer task)**.

---

See the main rationale and dead-lock explanation in
[backpressure_investigation.md](mdc:backpressure_investigation.md).