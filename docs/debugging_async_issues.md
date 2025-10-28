# Debugging Async Issues with tokio-console

This guide covers how to diagnose and debug async runtime issues in the groups_relay using tokio-console.

## Table of Contents

1. [Quick Start](#quick-start)
2. [Understanding Lost Wakers](#understanding-lost-wakers)
3. [tokio-console Interface](#tokio-console-interface)
4. [Common Debugging Workflows](#common-debugging-workflows)
5. [Capturing Diagnostics](#capturing-diagnostics)
6. [Common Issues and Solutions](#common-issues-and-solutions)

## Quick Start

### Local Development

```bash
# Build with console feature enabled
cargo run --features console

# In another terminal, connect to tokio-console
tokio-console http://localhost:6669
```

### Production/Remote Server

```bash
# SSH into the server
ssh communities

# Connect to tokio-console (already configured with console feature)
source ~/.cargo/env
tokio-console http://localhost:6669
```

### Docker Environment

```bash
# The production docker-compose.yml already exposes port 6669
# From the server:
tokio-console http://localhost:6669
```

## Understanding Lost Wakers

### What Are Lost Wakers?

"Lost wakers" occur when:
1. An async task is woken (notified it can make progress)
2. The task is scheduled to run by the tokio runtime
3. The task gets dropped/cancelled before it actually runs
4. The waker notification is "lost" because the task never executed

### Why Are They Concerning?

- **Resource waste**: CPU cycles spent waking tasks that never run
- **Performance impact**: Scheduler overhead for no productive work
- **Potential bugs**: May indicate race conditions or improper cancellation
- **Design smell**: Often points to suboptimal async patterns

### Common Causes

1. **Timeout cancellations** - Tasks cancelled by timeouts while queued
2. **Client disconnections** - Connection drops during request processing
3. **select! races** - Tasks cancelled when other branches complete first
4. **Dropped JoinHandles** - Tasks aborted when handles are dropped
5. **Channel receiver drops** - Senders waking when receiver is gone

### Are They Always Bad?

Not necessarily! Some lost wakers are expected:
- Client disconnections during long-polling are normal
- Timeout-protected operations may legitimately cancel
- select! patterns intentionally cancel losing branches

**Rule of thumb**: A few lost wakers (1-5%) is normal. Many lost wakers (>10%) indicates a problem.

## tokio-console Interface

### Views

Press these keys to switch views:
- `t` - Tasks view (default)
- `r` - Resources view (mutexes, semaphores, etc.)
- `a` - Async operations view

### Tasks View Controls

- `↑↓` or `k,j` - Scroll through tasks
- `←→` or `h,l` - Select different columns for sorting
- `i` - Invert sort order (highest/lowest)
- `Enter` - View detailed task information
- `/` - Search/filter tasks
- `space` - Pause/unpause updates
- `q` - Quit

### Important Columns

- **Warn** - Shows `⚠` if task has warnings (lost wakers, large size, etc.)
- **ID** - Unique task identifier
- **State** - `▶ Running` or `⏸ Idle`
- **Total** - Total time since task was spawned
- **Busy** - Time spent actively running (CPU time)
- **Polls** - Number of times the task was polled
- **Location** - Where the task was spawned in the code

### Detailed Task View

Press `Enter` on a task to see:
- Full spawn location with file:line
- Waker operations (wakes, wake-ops, self-wakes)
- Poll statistics (min, max, mean, p50, p90, p99)
- Task fields and state
- Complete task history

## Common Debugging Workflows

### Workflow 1: Investigating Server Hang

When the server appears hung:

```bash
# 1. Connect to tokio-console
ssh communities
source ~/.cargo/env
tokio-console http://localhost:6669

# 2. Look for tasks with unusual characteristics:
#    - Very high poll counts (spinning)
#    - Long total time with minimal busy time (blocked)
#    - Running state for extended periods
#    - Lost wakers warnings

# 3. Press Enter on suspicious tasks to see:
#    - Where they were spawned (Location)
#    - What they're waiting on (State)
#    - Waker activity

# 4. Capture the state for analysis
#    (See "Capturing Diagnostics" section)
```

### Workflow 2: Diagnosing Lost Wakers

```bash
# 1. Connect and switch to tasks view
tokio-console http://localhost:6669
# Press 't' for tasks view

# 2. Sort by warnings
# Press 'h' or 'l' to move to the "Warn" column
# Look for tasks marked with ⚠

# 3. Inspect each warned task
# Press Enter to see details
# Note the Location (where spawned) and poll frequency

# 4. Common patterns:
#    - Many warned tasks from same location = systematic issue
#    - High poll count + lost wakers = task spinning then cancelled
#    - Low poll count + lost wakers = task cancelled while queued
```

### Workflow 3: Performance Profiling

```bash
# 1. Connect and let it run for a few minutes
tokio-console http://localhost:6669

# 2. Sort by "Busy" time
# Press 'h' or 'l' until "Busy" column is highlighted
# Press 'i' to sort by highest

# 3. Identify CPU-heavy tasks
# Tasks with high Busy/Total ratio are CPU-bound
# Tasks with low Busy/Total ratio are I/O-bound

# 4. Check poll statistics
# Press Enter on high-CPU tasks
# Look for:
#    - High max poll duration = blocking operation
#    - High mean poll duration = consistently slow
#    - High poll count = task spinning
```

## Capturing Diagnostics

### Using tmux (Recommended)

```bash
# Start a tmux session
ssh communities
tmux new-session -s diagnosis

# Run tokio-console
source ~/.cargo/env
tokio-console http://localhost:6669

# From another terminal/pane, capture the screen
tmux capture-pane -t diagnosis -p -S - > tokio-console-$(date +%Y%m%d-%H%M%S).txt

# When done
tmux kill-session -t diagnosis
```

### Automated Capture

Use the provided script:

```bash
./scripts/diagnose_tokio_console.sh
```

This will:
- Connect to tokio-console
- Wait a few seconds to gather data
- Capture the current state
- Save to a timestamped file

### Manual Snapshot

```bash
# Start console in background
tmux new-session -d -s tokio_snapshot "source ~/.cargo/env && tokio-console http://localhost:6669"

# Wait for connection
sleep 3

# Capture and save
tmux capture-pane -t tokio_snapshot -p -S - > snapshot.txt

# Cleanup
tmux kill-session -t tokio_snapshot

# View the snapshot
cat snapshot.txt
```

## Common Issues and Solutions

### Issue: Many Lost Wakers from Same Location

**Symptoms:**
- 10+ tasks with lost waker warnings
- All from same spawn location
- High poll counts

**Likely Cause:**
- Timeout layer cancelling tasks
- Client disconnections during processing
- Improper task cancellation

**Solution:**
1. Check if timeouts are too aggressive
2. Verify timeout is not applied to WebSocket routes
3. Add graceful cancellation handling
4. Consider if the pattern is actually acceptable (client disconnects)

### Issue: Task with Very High Poll Count

**Symptoms:**
- Single task with 1000+ polls
- Low busy time relative to total time
- May have lost waker warning

**Likely Cause:**
- Spinning/busy-wait loop
- Incorrect async usage (blocking in async)
- Missing `.await` causing tight loop

**Solution:**
1. Find spawn location (Location field)
2. Review code for busy-wait patterns
3. Add proper `.await` points
4. Consider using `tokio::task::yield_now()` if intentional

### Issue: Task Stuck in Running State

**Symptoms:**
- Task shows `▶ Running` for extended period
- High busy time
- Not making progress

**Likely Cause:**
- CPU-bound computation without yield points
- Blocking operation in async context
- Deadlock on synchronization primitive

**Solution:**
1. Move CPU-intensive work to `spawn_blocking`
2. Add yield points with `tokio::task::yield_now()`
3. Check for lock contention (switch to Resources view)
4. Review for potential deadlocks

### Issue: Unexpected Task Count

**Symptoms:**
- More tasks than expected
- Tasks not being cleaned up
- Memory growth over time

**Likely Cause:**
- Task leak (spawned but not joined/aborted)
- Missing cancellation on shutdown
- Long-lived background tasks

**Solution:**
1. Review task spawn locations
2. Ensure proper JoinHandle management
3. Implement graceful shutdown
4. Use `tokio::task::JoinSet` for managed task groups

## Advanced: Interpreting Waker Statistics

In the detailed task view, you'll see waker stats:

```
Waker Operations
┌─────────────┬─────────┐
│ Wakes       │ 143     │
│ Wake Ops    │ 156     │
│ Self Wakes  │ 13      │
│ Waker Drops │ 0       │
│ Waker Clones│ 143     │
└─────────────┴─────────┘
```

**What they mean:**
- **Wakes**: Number of times the task was woken
- **Wake Ops**: Total waker operations (may include duplicates)
- **Self Wakes**: Task woke itself (common in futures that check state)
- **Waker Drops**: Wakers dropped without waking (potential issue if high)
- **Waker Clones**: Waker was cloned (normal for shared state)

**Red flags:**
- **Waker Drops > Wakes**: Many wakers dropped without use
- **Self Wakes very high**: Task might be spinning
- **Wakes >> Polls**: Task being woken but not scheduled (backpressure)

## Performance Impact

Running with tokio-console enabled has overhead:
- **~10-20% CPU overhead** from instrumentation
- **Memory overhead** for task history (configured to 1 hour retention)
- **Network**: Small amount for console connection

**Recommendations:**
- Use in development and staging freely
- Enable in production only for debugging (already configured)
- Production overhead is acceptable for diagnostic capability

## Further Reading

- [tokio-console GitHub](https://github.com/tokio-rs/console)
- [Tokio documentation on debugging](https://tokio.rs/tokio/topics/tracing)
- [Understanding Wakers in Async Rust](https://rust-lang.github.io/async-book/02_execution/03_wakeups.html)
