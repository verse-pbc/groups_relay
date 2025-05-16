---
description: Rules for Getting Unstuck
globs: 
---
These rules provide a structured, meta-level approach to solving complex programming problems. They emphasize understanding, reflection, and incremental progress to reduce frustration and ensure effective solutions.

## Rules

### 1. Gather Context Before Coding

- **Action**: When facing a complex change, first read the relevant code, review examples, and study existing tests.
- **Steps**:
  - Understand the current implementation and its purpose.
  - Look for similar implementations or patterns in the codebase.
  - Review test cases to see expected behavior.
- **Purpose**: Ensures a solid understanding of the problem and existing solutions, preventing premature or misguided changes.
- **Connection to Debugging**: Aligns with starting at the failure point by grounding your approach in concrete information.

### 2. Reflect and Hypothesize

- **Action**: When stuck, brainstorm 5-7 possible causes of the issue.
- **Steps**:
  - List potential sources of the problem (e.g., incorrect assumptions, missing edge cases, external dependencies).
  - Distill these to 1-2 most likely causes based on evidence or intuition.
  - Use the debugging methodology [backwards_callstack_debugging.md](mdc:.roo/rules/backwards_callstack_debugging.md) to validate your hypotheses.
- **Purpose**: Encourages systematic thinking and reduces the risk of chasing the wrong problem.
- **Connection to Debugging**: Applies ternary state classification (✅, ❌, ❓) and backward tracing to test assumptions.

### 3. Implement Incrementally

- **Action**: Make small, verifiable changes, prioritizing simplicity.
- **Steps**:
  - Break down the solution into small steps.
  - After each change, validate with tests, compilation (`cargo check`), or log verification.
  - Commit changes after each successful step.
- **Purpose**: Reduces complexity, isolates issues, and ensures steady progress.
- **Connection to Debugging**: Mirrors branch investigation and verification through testing in the debugging methodology.

### 4. Validate and Document

- **Action**: After each incremental change, stop to validate and document.
- **Steps**:
  - Run `cargo check` to ensure the code compiles.
  - Execute relevant tests to confirm behavior.
  - Review logs or outputs to verify correctness.
  - Commit changes with clear messages describing the fix or progress.
- **Purpose**: Creates a clear trail of validated steps, making it easier to backtrack if needed.
- **Connection to Debugging**: Aligns with the emphasis on documentation and verification in the debugging methodology.

---

## Practical Example

Suppose you’re implementing a Rust feature and the program panics unexpectedly.

1. **Gather Context**:
   - Read the code around the panic.
   - Check similar functions in the codebase.
   - Review tests for expected behavior.

2. **Reflect and Hypothesize**:
   - Brainstorm causes: incorrect input, uninitialized variables, race conditions, etc.
   - Narrow to "incorrect input" as the likely cause.
   - Add logs before the panic to trace inputs using `@backwards_callstack_debugging.md`.

3. **Implement Incrementally**:
   - Add input validation:
     ```rust
     if input.is_empty() { return Err("Empty input"); }
     ```
   - Run `cargo check` and a new test:
     ```rust
     #[test]
     fn test_empty_input() { assert!(process_input("").is_err()); }
     ```
   - Commit: "Added input validation to prevent panic."

4. **Validate and Document**:
   - Run tests to confirm the fix.
   - Check logs to verify input handling.
   - Document in the commit message.

---

## Key Benefits

- **Clarity**: Actionable steps make problem-solving straightforward.
- **Efficiency**: Understanding first minimizes wasted effort.
- **Reproducibility**: Incremental changes and documentation ensure a traceable process.
- **Debugging Integration**: Seamlessly connects to systematic debugging practices.