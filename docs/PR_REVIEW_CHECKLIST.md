# PR review checklist for AtomicDEX-API

- [ ] Check that all CI build stages passed successfully. It's acceptable to have unstable tests failing. If you are unsure whether
the test is unstable, please clarify it with the team.
- [ ] For new features: check that implementation matches feature specification (if there's any).
- [ ] For bugs: check that implementation actually fixes the bug.
- [ ] If you are unsure about code correctness, checkout it locally and test a possible problematic case (unit or manually).
- [ ] Ensure that the code is properly tested. Bugs should be covered to avoid future regression. It's allowed to skip tests for non-critical code if it's *hard* to test it.
- [ ] Check the code for potential bugs (deadlocks, mem leaks, panics, logic bugs, security problems).
- [ ] Check that unwrap/expect uses are properly justified.
- [ ] Check that naming actually reflects what the code is doing.
- [ ] Indicate code that is worth moving to a separate module or crate.
- [ ] Check if the code can be improved/simplified: it might be overly abstracted or require the additional abstraction layer instead for a better design.
- [ ] Follow SOLID if applicable.
- [ ] For PRs targeting release (mm2.1) branch check that QA tested and approved it.