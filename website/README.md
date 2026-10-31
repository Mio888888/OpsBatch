# OpsBatch website / 官网

**English** · [简体中文](#简体中文)

A standalone static site for OpsBatch: product overview and downloads. It does not touch the desktop app. The copy
follows the current Rust + GPUI build and should be kept in step with [`README.md`](../README.md) and
[`README_CN.md`](../README_CN.md) so the site never promises more than the app does.

Published at <https://mio888888.github.io/OpsBatch/>. The page is Chinese and English (switch at the top right; it
follows the browser language the first time).

## Preview

```sh
cd website
python3 -m http.server 4173
```

Open <http://localhost:4173>.

## Publishing

[`.github/workflows/website.yml`](../.github/workflows/website.yml) deploys `website/` to GitHub Pages whenever it
changes on `master`. Enable it once under *Settings → Pages → Build and deployment → Source → GitHub Actions*.

## Downloads

The page detects the visitor's system and reads the latest release from the GitHub API (`api.github.com` allows
cross-origin reads; files on `github.com/.../releases/download` do not). It picks these assets:

| System | Asset |
| --- | --- |
| macOS | `OpsBatch-<version>-macos-aarch64.dmg`, else `-x86_64.dmg` |
| Windows | `OpsBatch-<version>-windows-x86_64-setup.exe` |
| Linux | `OpsBatch-<version>-linux-x86_64.AppImage`, else the `.deb` |

If the API is unreachable it falls back to the releases page and the version in `index.html` (`fallbackVersion` and the
version stat; raise them with each release).

## Editing the copy

Chinese is the text in `index.html`; English lives in the `i18n.en.text` dictionary in the script at the bottom, keyed
by the exact Chinese string. Change both together, or the English page shows the Chinese text.

## Screenshots

`assets/screenshots/` still holds captures from the Tauri build. The GPUI interface was built to match it, but replace
them with captures of the current app when you can; keep the file names, or update the carousel in `index.html`.

## 简体中文

OpsBatch 的独立静态官网：产品介绍与下载，不影响桌面端。文案以当前 Rust + GPUI 版本为准，请与
[`README.md`](../README.md)、[`README_CN.md`](../README_CN.md) 一起维护，避免官网承诺超出应用实际能力。

线上地址：<https://mio888888.github.io/OpsBatch/>。页面支持中英文（右上角切换，首次按浏览器语言选择）。

**预览**：`cd website && python3 -m http.server 4173`，打开 <http://localhost:4173>。

**发布**：`master` 上 `website/` 有变更时，[`website.yml`](../.github/workflows/website.yml) 会部署到 GitHub Pages；
首次使用需在 *Settings → Pages → Source* 选择 *GitHub Actions*。

**下载**：页面识别访问者系统，从 GitHub API 读取最新发布（`api.github.com` 允许跨域，`github.com` 的下载地址不允许），
匹配上表中的安装包；读取失败时回退到发布页和 `index.html` 里的版本号（`fallbackVersion` 与版本统计，每次发版时请同步）。

**改文案**：`index.html` 里的中文是原文，英文在底部脚本的 `i18n.en.text` 词典中，以完整中文字符串为键。两处要一起改，否则英文页会显示中文。

**截图**：`assets/screenshots/` 目前仍是 Tauri 版的截图。GPUI 界面按它 1:1 复刻，但建议尽快换成当前应用的截图，保持文件名不变即可。
