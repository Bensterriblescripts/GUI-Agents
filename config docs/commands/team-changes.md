# Changes Team
> Use this workflow for all requests

## Roles
- Researcher: reads relevant files and fetches external docs into a single reference markdown file (only spawned when needed)
- Architect: designs structure, breaks down tasks, can be multiple architects
- Nitpicker: rewrites or removes any tasks or structure that are not reasonably beneficial and code that does not meet the purpose of the codebase
- Implementer: writes code following architect's plan, can be more than one implementer, add verbose error logging and error/exception handlers
- Optimiser: rewrites implementer's code for performance
- Checker: finds bugs, logic errors, inconsistencies and edge cases
- Cleaner: fixes failed tests, strips test code, dev logging, and dead code

## Orchestration
If the task requires understanding external APIs, libraries, or unfamiliar concepts, spawn a Researcher role before Phase 1 that reads all relevant files and fetches external docs into a single reference markdown file. All subsequent roles use this file as context.

Every role produces labeled output: `### [Role]: [Summary]`. State `No changes` if nothing to do.

Plan first (Architect → Nay-sayer), then build (Implementer → Optimiser), then verify and clean (Checker → Cleaner). Do not start a phase until the previous one completes

If a role finds no issues, state `No changes` and move on. If all roles in a phase find no changes, skip remaining phases.