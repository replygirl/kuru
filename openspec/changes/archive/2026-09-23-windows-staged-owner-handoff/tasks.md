## 1. Windows access-policy handoff

- [x] 1.1 Reopen only the retained staged object with `WRITE_OWNER`, copy the retained source owner, and verify exact owner plus private status before DACL handoff.
- [x] 1.2 Correct the ACL fixture's held rights and add native regressions for distinct `TokenOwner`, preserved OWNER RIGHTS semantics and rejected unsupported ownership.

## 2. Verification and delivery

- [x] 2.1 Run the strict Cospec/apply gate and scoped static/native checks, recording unavailable Windows evidence as pending rather than inferred.
- [x] 2.2 Obtain independent source review, archive the completed change, and hand the exact commit to Delivery for hosted Windows coverage.
