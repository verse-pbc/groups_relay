---
description: Best practices for commenting Rust code
globs: ["*.rs"]
---

This rule outlines best practices for commenting Rust code, ensuring clarity and consistency for both developers and AI-assisted coding.

### Guidelines for Developers

- **Regular Comments (`//`)**:
  - Use these to explain *why* the code is written a certain way, rather than *what* it does.
  - Keep them concise and place them above the code they describe.
  - **Example**:
    ```rust
    // Use a loop to avoid stack overflow for large inputs.
    fn factorial(n: u32) -> u32 {
        let mut result = 1;
        for i in 1..=n {
            result *= i;
        }
        result
    }
    ```

- **Documentation Comments (`///`)**:
  - Use these for public APIs to generate documentation with `rustdoc`.
  - Start with a single-line summary in third person, using American English.
  - Include relevant sections such as "Examples," "Panics," "Errors," etc., as needed.
  - Prefer `///` over `/** */` for documentation, following RFC 1574 conventions.
  - **Example**:
    ```rust
    /// Calculates the factorial of a number.
    ///
    /// # Panics
    ///
    /// Panics if the input is negative.
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(120, factorial(5));
    /// ```
    pub fn factorial(n: u32) -> u32 {
        // Implementation
    }
    ```

- **General Tips**:
  - Ensure comments are clear, concise, and up-to-date with the current code.
  - Avoid over-commenting; let the code be self-explanatory where possible.
  - Use American English consistently, especially in documentation comments.

### For AI Assistance

When generating or suggesting Rust code, the AI should include comments that adhere to these practices:
- For public items (functions, structs, enums, etc.), provide `///` documentation comments with a summary and relevant sections (e.g., "Examples" or "Panics").
- For complex or non-obvious logic, add `//` comments to explain the reasoning or intent behind the code.

### Why Follow These Guidelines?

By adhering to these best practices, your Rust code will be more readable, maintainable, and aligned with community standards, benefiting both developers and users of your APIs.
