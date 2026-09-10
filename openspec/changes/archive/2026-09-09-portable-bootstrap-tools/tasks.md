## 1. Preserve the system checksum launch path

- [x] 1.1 Replace the relocated checksum symlink in bootstrap_install.rs with an absolute-path exec wrapper; verify all thirteen real Bash tests still exercise the host checksum implementation and original assertions. The installer task passed all thirteen bootstrap tests in 2.28 seconds, /tmp/kuru-portable-bootstrap-tests.log. Independent source review confirmed the Apple launcher behavior and that no assertion, checksum implementation or compiler-free PATH constraint changed.
- [x] 1.2 Run the full repository gate, record the observed result, and archive through cospec before committing; observe all platform PR gates on the committed repair before merge. Full mise check passed in 53.79 seconds with 97.46% line coverage (8993/9227), /tmp/kuru-portable-bootstrap-check.log. The original head passed its other three platform jobs; the macOS failure remains recorded. Fresh hosted checks require this archived repair to be committed and pushed and will be observed before merge.

Primary mechanism evidence: https://github.com/apple-oss-distributions/perl/blob/perl-175/versioner/versioner.c#L194
and https://github.com/apple-oss-distributions/perl/blob/perl-175/fix/dummy.pl#L15.
