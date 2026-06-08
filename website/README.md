# OpsBatch Website

这是 OpsBatch 的独立静态官网，不接入现有 Tauri/React 应用入口，也不改变桌面端架构。页面面向产品介绍与软件下载，会根据访问者系统自动推荐 macOS、Windows 或 Linux 版本。

## 本地预览

```bash
cd website
python3 -m http.server 4173
```

打开 `http://localhost:4173` 即可预览。

## 下载链接

当前下载按钮指向：

```text
https://github.com/Mio888888/OpsBatch/releases/latest
```

如果后续更换仓库或发布地址，请同步修改 `index.html` 中的下载链接。静态页不会硬编码具体安装包文件名，避免 Release 产物命名变化导致链接失效。
