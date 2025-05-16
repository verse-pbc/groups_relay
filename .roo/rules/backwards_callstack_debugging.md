---
description: Debugging Methodology: Rules for Systematic Issue Resolution
globs: 
---
# Debugging Methodology

This methodology provides a structured, log-driven approach to identify and resolve software issues by tracing from failure points back to their root causes. It uses a ternary state classification system to eliminate ambiguity and focus debugging efforts. Follow these rules to apply the methodology effectively.

## Rules for the LLM to Follow

### 1. Start at the Failure Point

- **Action:** Begin debugging where the issue is observed.
- **Steps:**
  - Record the exact location (e.g., function, line number).
  - Capture error messages, stack traces, and system state.
  - Document the expected behavior versus the actual behavior.
- **Purpose:** Establishes a concrete starting point for investigation.

### 2. Classify States Using Ternary Categorization

- **Action:** Use logging to categorize program states and events into three types:
  - ✅ **Confirmed:** States or events that definitely occurred (e.g., a function executed successfully).
  - ❌ **Negative:** States or events that definitely did not occur (e.g., a condition failed).
  - ❓ **Unknown:** States or events needing further investigation (e.g., unclear execution path).
- **Steps:**
  - Add initial logs at the failure point to classify the state.
  - Review logs to assign ✅, ❌, or ❓ labels.
- **Purpose:** Reduces speculation by grounding analysis in evidence.

### 3. Trace Backwards from Unknown States

- **Action:** Investigate each ❓ unknown state to determine its status.
- **Steps:**
  - Add detailed logging before the unknown state (e.g., input values, conditions).
  - Rerun the program to collect log data.
  - Classify the state as ✅ or ❌ based on the new logs.
  - Repeat, tracing backwards, until reaching a ✅ confirmed state.
- **Example:**  
  If a function fails (❓), log its inputs and preconditions with `println!`, then rerun to confirm execution. Avoid too much dumped data, so avoid big structs dumping and {:?} with raw variables
- **Purpose:** Builds a clear path from the failure to earlier states.

### 4. Investigate Execution Branches

- **Action:** Examine paths from ✅ confirmed states to ensure correct flow.
- **Steps:**
  - Document the execution path leading to the ✅ state.
  - Verify it matches the expected logic (e.g., correct function calls, conditions).
  - If the path deviates or breaks, start a new trace from the deviation point.
- **Example:**  
  From a ✅ "data validated" state, check if the next step ("transform data") occurs as expected.
- **Purpose:** Ensures all branches align with intended behavior.

### 5. Identify the Root Cause

- **Action:** Pinpoint where the failure originates.
- **Steps:**
  - Look for where a ✅ confirmed good state:
    - Fails to reach the next expected state, or
    - Transitions directly to a ❌ known bad state.
  - Mark this transition as the root cause.
- **Example:**  
  A ✅ "file opened" state leads to a ❌ "write failed" state due to a permission issue.
- **Purpose:** Isolates the exact point of failure for resolution.

### 6. Implement Trace Points

- **Action:** Add detailed logging to capture state and flow.
- **Steps:**
  - Insert logs at key points (e.g., function entry/exit, condition checks).
  - **Tip:** Prioritize using `println!` over traditional logging macros (like `debug!` or `info!`) because `println!` is easier to set up, its output is simple to grep for debugging artifacts, and it can be quickly removed once debugging is complete.
  - Use a consistent format, such as:

    ```rust
    println!("[Module::function] State: {:?}", current_state);
    ```

  - Log transitions and results:

    ```rust
    println!("[Module::transition] From {} to {}", old_state, new_state);
    ```
- **Purpose:** Provides data to classify states and trace execution.

### 7. Verify Through Testing

- **Action:** Run tests with enhanced logging to validate the trace.
- **Steps:**
  - Execute the program with the added `println!` statements.
  - Grep the console output as needed to identify gaps (❓ states) and refine trace points.
- **Purpose:** Ensures the methodology produces actionable insights.

### 8. Document the Process

- **Action:** Record findings for reproducibility and reference.
- **Steps:**
  - Summarize the failure point, state classifications, and root cause.
  - Note all trace points added and their outcomes.
  - Save logs and conclusions in a clear format.
- **Purpose:** Creates a trail for future debugging or team collaboration.

## Practical Example in Rust

```rust
// Original code with unclear failure
fn process_input(&self, input: &str) -> Result<(), String> {
    self.check_input(input)?;
    self.store(input)
}

// Debuggable version with trace points using println!
fn process_input(&self, input: &str) -> Result<(), String> {
    println!("[process_input] Starting with input: {}", input);

    println!("[process_input] Checking input");
    match self.check_input(input) {
        Ok(_) => println!("[process_input] ✅ Input valid"),
        Err(e) => {
            println!("[process_input] ❌ Input check failed: {}", e);
            return Err(e);
        }
    }

    println!("[process_input] Storing input");
    self.store(input)
}
```

- **Failure:** `store` fails unexpectedly (❓).
- **Trace Back:** Add `println!` statements to `check_input` to confirm its output (✅ or ❌).
- **Root Cause:** Found if `check_input` passes a bad value to `store`.

## Key Benefits

- **Clarity:** Ternary classification eliminates guesswork.
- **Efficiency:** Focuses effort on ❓ unknown states.
- **Reproducibility:** Simple logging with `println!` creates a repeatable process.

## Applying the Rules: Step-by-Step Summary

1. Locate the failure and log its details.
2. Classify initial states as ✅, ❌, or ❓ using `println!` logs.
3. Trace ❓ states backwards with new logs until reaching ✅ states.
4. Check branches from ✅ states for expected flow.
5. Find the root cause at a ✅ to ❌ transition or broken expectation.
6. Add `println!` trace points to capture critical data.
7. Test with the enhanced logging to verify the trace.
8. Document everything for clarity and future use.