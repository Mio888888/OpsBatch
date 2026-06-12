# OpsBatch Website

这是 OpsBatch 的独立静态官网，不接入现有 Tauri/React 应用入口，也不改变桌面端架构。页面面向产品介绍与软件下载，按当前代码中的主机资产、SSH/SFTP、RDP/VNC、批量执行、工作流、AI、日志和本地数据能力编写，会根据访问者系统自动推荐 macOS、Windows 或 Linux 版本。

## 本地预览

```bash
cd website
python3 -m http.server 4173
```

打开 `http://localhost:4173` 即可预览。

## 自动发布

官网通过 GitHub Actions 自动发布到 GitHub Pages。修改 `website/` 目录或
`.github/workflows/website.yml` 后推送到 `main` / `master`，会触发 `Website`
workflow，把 `website/` 作为静态站点发布。

首次启用时，需要在 GitHub 仓库中打开：

```text
Settings -> Pages -> Build and deployment -> Source -> GitHub Actions
```

如果希望 workflow 自动完成首次启用，可以在仓库 Secrets 中新增
`PAGES_TOKEN`，使用具备 Pages 写入权限的 Personal Access Token。未配置该
secret 时，workflow 会使用默认 `GITHUB_TOKEN`，但 GitHub 不允许它自动创建
Pages 站点。

发布地址沿用页面中的 canonical URL：

```text
https://mio888888.github.io/OpsBatch/
```

## 下载链接

当前下载按钮指向：

```text
https://github.com/Mio888888/OpsBatch/releases/latest
```

如果后续更换仓库或发布地址，请同步修改 `index.html` 中的下载链接。静态页不会硬编码具体安装包文件名，避免 Release 产物命名变化导致链接失效。

页面中的产品版本、功能范围和合规说明应随 `README.md`、`README_CN.md` 与 `README_EN.md` 一起维护，避免官网与实际桌面端能力脱节。
