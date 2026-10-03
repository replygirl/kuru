# Spec Delta

## ADDED Requirements

### Requirement: Activity record settled before the endpoint retires

An owner that published a tagged open activity record SHALL, in its close, await its publisher's last write after it stops accepting and before it retires its endpoint. Once its endpoint is retired, the record SHALL change only by its failing mark or its retirement. That wait SHALL add no retry, sleep or new bound: the publisher writes until the record holds the latest activity, which no longer changes once the open has ended and its counter is quiet, and it never rewrites after a failed write once the open has ended, and the owner's retirement already awaited the same publisher. A failed or unwritten last write SHALL NOT fail the close or change its order. The failing mark of an owner that ends before its starter attached SHALL still be written from the publisher's last activity after that wait.

#### Scenario: Last write pending when the close begins
- **WHEN** the publisher's last write after its open has not completed when the owner stops accepting
- **THEN** the owner retires its endpoint only after that write completes or fails, and a read of the record after the endpoint retires returns the open's last activity until the record is retired.

#### Scenario: Unserved owner after the wait
- **WHEN** an owner whose loop failed before its starter attached has awaited its publisher before retiring its endpoint
- **THEN** it still marks the record failing with that failure's reason before its store closes, and retires the record after the close.
