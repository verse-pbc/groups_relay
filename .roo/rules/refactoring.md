---
description:
globs:
alwaysApply: false
---
- **Upgrade & Refactoring Verification Rule**
  - **Objective:**
    - Ensure all affected files and their tests remain functionally equivalent after any library upgrade or major refactor, unless a change is explicitly required by the new library version or refactor objective.
    - Institutionalize a repeatable, knowledge-driven process for all future upgrades/refactors.

  - **Step 1: Gather and Document Upgrade Knowledge**
    - Use tools like `scan_crate` (or equivalents) to extract API changes, new patterns, and deprecations from the new library version.
    - Review official changelogs, migration guides, and any relevant external documentation.
    - Gather any extra documentation, notes, or lessons learned from previous upgrades or from the team.
    - Create or update a knowledge file (e.g., `upgrade_adaptation_knowledge.md`):
      - Document all observed API differences, required adaptations, and rationale for changes.
      - Include before/after code snippets, patterns, and explanations for tricky or non-obvious changes.
      - This knowledge file should serve as a living reference for the current and all future upgrades.

  - **Step 2: Identify and Gather Context for Each File**
    - Locate both the current (post-upgrade/refactor) and original (pre-upgrade/refactor, e.g., from a specific commit) versions of each affected file.
    - Reference the knowledge file for insights, patterns, and rationale about API changes and lessons learned during the upgrade or refactor.

  - **Step 3: Compare Test Coverage**
    - Extract all test functions from both the original and upgraded/refactored versions.
    - Ensure every test present in the original is also present in the upgraded/refactored file.
    - Confirm that no tests have been removed, commented out, or made less strict.
    - Check that the scope and intent of each test remain unchanged.

  - **Step 4: Compare Test Functionality**
    - Review the logic of each test to ensure it still checks the same behaviors and business rules.
    - Confirm that any changes are only due to necessary API or structural adjustments (e.g., type changes, new wrappers, lifetimes), not changes in what is being tested.

  - **Step 5: Compare the Code Under Test (Implementation)**
    - Carefully review the implementation code (not just the tests) in both the current and original versions.
    - Ensure that the core logic, control flow, and business rules enforced by the code remain functionally equivalent.
    - Use the knowledge file to validate that all required adaptations (e.g., new API usage, changed types, new error handling) are present and correct, and that no accidental logic drift or regression has occurred.
    - Confirm that any changes in the implementation are justified by the knowledge file, official migration requirements, or documented rationale—not by accidental or unnecessary rewrites.

  - **Step 6: Confirm No Functional Drift**
    - Ensure the intent and effect of both the tests and the code under test are unchanged: the same scenarios are covered, and the same assertions and behaviors are enforced.
    - Check that the core logic (as exercised by the tests) still enforces the same requirements as before.

  - **Step 7: Summarize and Document Findings**
    - Clearly state whether all original tests and their functionality are preserved.
    - Clearly state whether the implementation is functionally equivalent, with any differences justified by the knowledge file or migration requirements.
    - Note any differences and justify them (e.g., required by new API, not a loss of coverage or behavior).
    - Update the knowledge file with any new patterns, lessons, or edge cases discovered during this process.

  - **Step 8: Mark the Task as Done**
    - If all tests and implementation logic are preserved and functionally equivalent, and all findings are documented, mark the upgrade/refactor verification task as complete.

- **Best Practices:**
  - Always begin by gathering and updating the knowledge file for the new library version or refactor, using tools like `scan_crate`, changelogs, and any extra documentation.
  - Systematically apply this pattern to each affected file, comparing with the originals and documenting all findings and justifications for future reference.

- **Examples:**
  - See [upgrade_adaptation_knowledge.md](mdc:scripts/upgrade_adaptation_knowledge.md) for a living example of a knowledge file.
  - Example commit message: `fix(module): Upgrade to new library version, preserve test coverage and logic\n\n- All tests verified\n- Knowledge file updated with new patterns`
