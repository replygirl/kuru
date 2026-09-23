## 1. Effective post-owner privacy [critical]

- [ ] 1.1 @regression (agent) assign a distinct supported TokenOwner to a retained private stage on native Windows -> the resulting owner is exact, the protected TokenUser allow ACE remains, OWNER RIGHTS is an effective zero-mask ACE rather than inherit-only, and strict privacy validation passes
- [ ] 1.2 @integration (agent) exercise ordinary replacement, shielded publication and held source-DACL copying on native Windows -> publication preserves exact source ownership and access while every failure remains pre-publication

## 2. Unsupported ownership remains rejected [critical]

- [ ] 2.1 @regression (agent) attempt an unsupported staged owner assignment -> assignment fails without publication and the retained stage remains private under its original owner policy
