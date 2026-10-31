## Summary / 变更说明

<!-- What does this PR change, and why? / 这个 PR 改了什么，为什么需要？ -->

## Type / 变更类型

- [ ] Feature / 新功能
- [ ] Bug fix / 问题修复
- [ ] Refactor / 重构
- [ ] Performance / 性能
- [ ] Docs / 文档
- [ ] Tests / 测试
- [ ] Build, CI, chore / 构建、CI、杂项
- [ ] Breaking change (data, settings or update behaviour) / 破坏性变更（数据、设置或更新行为）

## Related issues / 关联 Issue

<!-- e.g. Closes #123 / 例如 Closes #123 -->

## How it was tested / 自测情况

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] Tried it in the running app (say on which OS) / 在运行的应用里手动验证过（注明系统）

<!-- Commands you ran, servers you connected to, anything not covered by tests. / 补充验证过程。 -->

## Screenshots / 截图

<!-- For UI changes, before and after. / 涉及界面时附上前后对比。 -->

## Checklist / 检查清单

- [ ] The title reads `area: what changed` (for example `VNC: pick the authentication type automatically`) / 标题形如 `模块: 做了什么`
- [ ] No secrets, tokens or real hostnames in code, tests or logs / 没有密钥、令牌或真实主机名
- [ ] New user-facing text is in both `locales/zh-CN.json` and `locales/en-US.json` / 新的界面文案已加入中英文词典
- [ ] Data stays compatible with the Tauri build (`0.x`) or the break is called out above / 数据与 Tauri 版保持兼容，否则已在上面说明
- [ ] `FEATURE_MATRIX.md` and the READMEs are updated if behaviour changed / 行为变化时已更新功能清单与 README
