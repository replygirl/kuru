## Why

The macOS 14 CI runner's shasum launcher resolves Perl-version alternatives from
its invocation path. The bootstrap fixture's temporary symlink changes that path,
so nine installation tests fail before reaching their intended assertions.

## What Changes

- Invoke the existing system checksum tool through an absolute-path exec wrapper
  in bootstrap_install.rs, preserving the fixture's compiler-free PATH.
- Retain every installation assertion and the real host checksum implementation.

## Impact

Test fixture setup only; no installer, dependency or workflow change. Run all
thirteen bootstrap behavior tests and the full gate, then observe the repaired
macOS 14 PR check before merge. Existing failed-job evidence is retained at
/tmp/kuru-direct-install-macos-failure.log (job 102739400916).

Apple's perl-175 versioner/versioner.c rewrites script paths only beneath the
interpreter's system prefix; fix/dummy.pl then searches for version siblings
using the invoked script path. Those upstream sources explain the hosted error
and support preserving the original absolute path. The local system shasum is a
direct Digest::SHA script, so its successful symlink invocation did not exercise
the older runner's launcher behavior.
