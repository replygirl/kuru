## 1. Post-owner DACL repair

- [x] 1.1 Factor the existing protected private DACL application from its prevalidation wrapper
- [x] 1.2 Reapply that exact DACL after supported exact-owner assignment through the retained `WRITE_DAC` handle, then preserve strict privacy and owner validation

## 2. Native regression and delivery

- [x] 2.1 Preserve the descriptor-shape regression that fails before the correction and proves effective OWNER RIGHTS after it
- [ ] 2.2 Run focused Windows platform and application fixtures, record observed evidence, validate and archive the change
