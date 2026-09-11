## 1. Preserve the failing connection phase

- [x] 1.1 Extend the existing real-Dolt identity rejection test to require the missing diagnostic, observe failure before implementation, then add bounded phase/cause context and verify it passes.
- [x] 1.2 Preserve startup and observed cleanup context, run the owning regression and existing backup/restore case, then verify native CI without changing budgets or checks.
- [x] 1.3 Record observed evidence and strictly validate the completed diagnostic fix; archive it before the final branch commit.
