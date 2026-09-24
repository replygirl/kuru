## 1. Effective post-owner privacy [critical]

Exact-head #71 CI 36004230622 at 89fc470 passed Windows platform job 107648153169 and application coverage job 107648153432. The platform's `staged_owner_assignment_retains_effective_owner_rights_privacy`, `ordinary_create_and_unprotected_replacement_regain_parent_acl_inheritance`, `replacement_retains_private_delete_authority_across_restrictive_target_dacl`, and retained-old access fixtures passed under native Windows; the application coverage shard also completed successfully. The aggregate native report and archive remain open.

Exact-head b200240 CI 36010519061 again passed native Windows platform job 107669710224 and application coverage job 107669711238, and the separate memory-runtime job 107669711038 passed 185/185 including the previously red warm-cache cleanup fixture. Windows aggregate report job 107682744513 passed the unchanged 90% gate with 63,977/69,192 covered lines (92.46%); the complete exact-head CI run succeeded. Final archive and merge follow this evidence checkpoint.

- [x] 1.1 @regression (agent) assign a distinct supported TokenOwner to a retained private stage on native Windows -> the resulting owner is exact, the protected TokenUser allow ACE remains, OWNER RIGHTS is an effective zero-mask ACE rather than inherit-only, and strict privacy validation passes
- [x] 1.2 @integration (agent) exercise ordinary replacement, shielded publication and held source-DACL copying on native Windows -> publication preserves exact source ownership and access while every failure remains pre-publication

## 2. Unsupported ownership remains rejected [critical]

- [x] 2.1 @regression (agent) attempt an unsupported staged owner assignment -> assignment fails without publication and the retained stage remains private under its original owner policy
