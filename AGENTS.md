# OpsBatch 项目协作约定

## 版本发布（Release）提交规范

每次更新版本号时，提交内容（commit body）必须包含**自上一个版本以来的全部变更**，而非仅写一句"更新版本至 x.y.z"。

### 操作步骤

1. **定位区间**：找到上一个版本提交的 hash，例如 `git log --oneline | grep -i release`，取最近一次。
   - 用 `git log --oneline <上一个版本hash>..HEAD` 列出区间内所有变更提交。
   - 区间**不包含**上一个版本提交本身，但**包含**本次版本提交之前的所有提交。

2. **逐项研读**：用 `git show --stat <hash>` 查看每个变更提交的标题、正文和改动文件，准确归纳用户可感知的变化（不是机械罗列提交）。

3. **按分类整理** changelog，分类与 0.2.0 范式一致（无内容的分类可省略）：
   - **新功能**：新增的能力、面板、入口
   - **界面重构**：UI 结构、布局、导航的重构
   - **问题修复**：bug 修复（标题去掉 `fix(scope):` 前缀，写成用户视角的描述）
   - **优化与维护**：性能、文案、构建、CI、依赖等

4. **同步版本号到 4 处**：
   - `package.json`
   - `src-tauri/Cargo.toml`
   - `src-tauri/tauri.conf.json`
   - `src-tauri/Cargo.lock`（仅 `opsbatch` 包自身的 `version =` 行，**不要**改第三方依赖的同名版本号）
   - 验证 `cargo check` 通过且 lockfile 不被改回。

5. **提交格式**：
   ```
   chore(release): 更新版本至 <新版本号>

   自 <上一个版本hash短> (v<上一版本>) 起的发布内容：

   <分类1>
   - <条目>

   <分类2>
   - <条目>

   同步应用、Tauri、Rust 与锁文件版本号至 <新版本号>
   ```

6. **安全性**：
   - 版本提交内容只描述本次版本区间内的变更，不混入历史。
   - `src-tauri/src/commands/app_update.rs` 中 `is_remote_version_newer("0.2.0", ...)` 等是版本比较测试用例字符串，**不要**当作项目版本号修改。
   - 若版本提交尚未推送，可用 `git commit --amend` 修订其内容；已推送的不可改。

### 范例

参见提交 `38714759`（v0.2.0）、`d9b0b8a6`（v0.2.1）的完整 body。
