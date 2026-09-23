## 1. Retained Windows replacement [critical]

- [ ] 1.1 @regression (agent) replace a checked regular destination while its validated data handle remains open on native Windows -> the call succeeds, the retained handle keeps the old identity and bytes, and a fresh destination open has the staged identity and bytes
- [ ] 1.2 @integration (agent) copy an unprotected ordinary file's access policy through retained-handle replacement and mutate the parent DACL afterward on native Windows -> the published file inherits identically to an ordinary sibling while the replaced handle remains bound to the old object
- [ ] 1.3 @regression (agent) replace a target whose copied DACL denies file DELETE while its parent authorizes child replacement -> a late DELETE reopen is denied, the retained private-stage authority publishes once, and the published file keeps the copied DACL

## 2. Publication boundaries

- [ ] 2.1 @regression (agent) exercise occupied new-only publication and invalid source/destination identity cases -> new-only still refuses without changing old bytes and mismatches reject before publication
- [ ] 2.2 @integration (agent) compile all `kuru-platform` targets and run its native Windows filesystem/security suite -> the handle-relative record, flags and error mapping compile and all platform contracts pass
