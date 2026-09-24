## 1. Vanished checked files remain rejected with accurate absence [critical]

- [x] 1.1 @regression (agent) unlink a checked Unix regular file while retaining its open handle, then validate that handle -> macOS platform filesystem integration passed 20/20; the retained handle reported zero links and validation rejected it as `io::ErrorKind::NotFound`
- [x] 1.2 @regression (agent) run the real independent managed-memory clients through warm reuse and endpoint retirement -> release run 35823820239 job 107064339047 failed before the fix with the conflated hardlink error; the corrected exact host fixture passed 1/1 in 32.54s with one generation and clean endpoint disappearance (hosted release rerun pending)

## 2. Additional hardlinks remain forbidden [critical]

- [x] 2.1 @unit (agent) validate a retained regular file that still has two names -> macOS platform filesystem integration passed 20/20; read, read-write, lock, and seal each rejected the two-link handle with `io::ErrorKind::PermissionDenied`
