# Dependencies

## Blocked by

None.

## Soft-blocked by

None.

## Siblings

- `shell-environment-minimization` owns the production environment projection
  whose native acceptance fixture is corrected here.
- `bounded-provider-retries` owns the production refresh budget whose native
  timing fixture is corrected here.
- `native-trust-publication` and `native-advisory-and-path-validation` address
  separate failures from the same Windows checkpoint run and share no files
  with this change except a disjoint block of `windows_terminal.rs`.
