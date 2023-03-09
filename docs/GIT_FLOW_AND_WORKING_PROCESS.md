# Git flow

1. There are two permanent branches: main (master/release) and dev.
2. The goal is to have both main and dev ready to be released at any point in time - all auto-tests pass, no incompatibilities introduced, etc.
3. The dev is merged to main when a new release is planned.
4. The feature branch lifetime should be <= 1-2 weeks.
5. Big task should be decomposed to the several feature branches. Each of which is merged periodically with a code review process. The new feature branch should be started from dev afterward.
6. In certain cases, the dev might not be in a "releasable" state. In this case, if the feature branch has no dependencies updates, it might be merged directly to main if required (hotfix, very useful feature, blocker). Dev is synced with main afterward.
7. For convenience, we can consider making several minor features/fixes in the single feature branch.

Pros:
1. Small intermediate PRs will be easier and faster to review.
2. Cross-merging all the ongoing work frequently will allow us to catch possible git conflicts sooner than later. Will be also easier to solve them.
3. Easier to track everyone's progress and provide help if required.

Cons:
1. Decomposing to multiple feature branches and constantly keeping backward compatibility might be a bit hard.

# Issues handling process. Development/QA/Documentation workflow.

1. It's desired to have a separate issue for any bug report or feature request.
2. Once the issue is created, add it to the MM2.0 Github project. Select an appropriate column.
3. Decide whether you should base your feature branch on main or dev. For hotfixes or minor useful features that don't include any dependencies updates choose main. In other cases choose dev.
5. PR titles must have a prefix that displays the current status of PR. Such as `[wip] X feat integration`, `[r2r] X feat integration`, where `[wip]` prefix stands for "Work in Progress", and `[r2r]` for Ready to Review.
4. PRs to dev can be merged right after approval. Request the tests in the dev branch from Tony by assigning the issue to him and moving it to the `Testing` column. Provide a detailed explanation of what changed and what should be tested. Indicate the critical points.
5. PRs to main must be tested by QA *before* merging.
6. If documentation update is required, prepare examples and notify smk762. Assign issue to him. Move the issue to the documentation column. Smk will then prepare PR in [developer-docs](https://github.com/KomodoPlatform/developer-docs) repo.
7. Review the docs PR. Smk will request it from the feature implementor.

# By this signature, I confirm that I read and understood this document  
[@artemii235](https://github.com/artemii235)
[@sergeyboyko0791](https://github.com/sergeyboyko0791)
[@shamardy](https://github.com/shamardy)
[@ozkanonur](https://github.com/ozkanonur)

