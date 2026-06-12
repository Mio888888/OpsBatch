const navLinks = [...document.querySelectorAll('.nav-links a')];
const sections = navLinks
  .map((link) => document.querySelector(link.getAttribute('href')))
  .filter(Boolean);

const releasesUrl = 'https://github.com/Mio888888/OpsBatch/releases/latest';
const latestReleaseApi = 'https://api.github.com/repos/Mio888888/OpsBatch/releases/latest';
const osLabels = {
  mac: 'macOS',
  windows: 'Windows',
  linux: 'Linux',
  unknown: '未知系统',
};
const assetMatchers = {
  mac: [/\.dmg$/i, /\.app\.tar\.gz$/i],
  windows: [/\.exe$/i, /\.msi$/i],
  linux: [/\.AppImage$/i, /\.deb$/i, /\.rpm$/i],
};

const detectOS = () => {
  const platform = (navigator.userAgentData?.platform || navigator.platform || '').toLowerCase();
  const agent = (navigator.userAgent || '').toLowerCase();
  const text = `${platform} ${agent}`;

  if (text.includes('mac') || text.includes('iphone') || text.includes('ipad')) return 'mac';
  if (text.includes('win')) return 'windows';
  if (text.includes('linux') || text.includes('x11')) return 'linux';
  return 'unknown';
};

const setActiveLink = () => {
  let active = sections[0];
  sections.forEach((section) => {
    if (section.getBoundingClientRect().top <= 130) active = section;
  });
  navLinks.forEach((link) => {
    const isActive = link.getAttribute('href') === `#${active.id}`;
    link.toggleAttribute('aria-current', isActive);
  });

  const mobileLinks = document.querySelectorAll('.mobile-nav-links a');
  mobileLinks.forEach((link) => {
    const isActive = link.getAttribute('href') === `#${active.id}`;
    link.toggleAttribute('aria-current', isActive);
  });
};

document.addEventListener('scroll', setActiveLink, { passive: true });
setActiveLink();

document.querySelectorAll('.clay-button').forEach((button) => {
  button.addEventListener('pointerdown', () => button.classList.add('is-pressed'));
  button.addEventListener('pointerup', () => button.classList.remove('is-pressed'));
  button.addEventListener('pointerleave', () => button.classList.remove('is-pressed'));
});

// Mobile nav toggle
const menuToggle = document.querySelector('.menu-toggle');
const mobileNav = document.querySelector('.mobile-nav');
const mobileOverlay = document.querySelector('.mobile-overlay');
const mobileNavClose = document.querySelector('.mobile-nav-close');

const openMobileNav = () => {
  mobileNav.classList.add('is-open');
  mobileOverlay.classList.add('is-visible');
  mobileNav.removeAttribute('aria-hidden');
  mobileOverlay.removeAttribute('aria-hidden');
  if (menuToggle) menuToggle.setAttribute('aria-expanded', 'true');
  document.body.style.overflow = 'hidden';
};

const closeMobileNav = () => {
  mobileNav.classList.remove('is-open');
  mobileOverlay.classList.remove('is-visible');
  if (menuToggle) menuToggle.setAttribute('aria-expanded', 'false');
  document.body.style.overflow = '';
};

if (menuToggle) menuToggle.addEventListener('click', openMobileNav);
if (mobileNavClose) mobileNavClose.addEventListener('click', closeMobileNav);
if (mobileOverlay) mobileOverlay.addEventListener('click', closeMobileNav);

document.querySelectorAll('.mobile-nav-links a, .mobile-nav-cta').forEach((link) => {
  link.addEventListener('click', () => {
    closeMobileNav();
  });
});

document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape' && mobileNav.classList.contains('is-open')) {
    closeMobileNav();
  }
});

const currentOS = detectOS();
const detectedOS = document.querySelector('#detected-os');
const primaryDownload = document.querySelector('#primary-download');
const downloadStatus = document.querySelector('#download-status');
const recommendedOption = document.querySelector(`.download-option[data-os="${currentOS}"]`);

if (detectedOS) {
  detectedOS.textContent = currentOS === 'unknown' ? '未能自动识别，请手动选择' : `${osLabels[currentOS]} 设备`;
}

if (primaryDownload) {
  primaryDownload.href = releasesUrl;
  primaryDownload.textContent = currentOS === 'unknown' ? '打开下载页' : `下载 ${osLabels[currentOS]} 版`;
}

if (recommendedOption) {
  recommendedOption.classList.add('is-recommended');
  recommendedOption.setAttribute('aria-label', `推荐下载 ${osLabels[currentOS]} 版本`);
}

const setDownloadStatus = (message, tone) => {
  if (!downloadStatus) return;
  downloadStatus.textContent = message;
  downloadStatus.classList.toggle('is-ready', tone === 'ready');
  downloadStatus.classList.toggle('is-fallback', tone === 'fallback');
};

const chooseAsset = (assets, os) => {
  const matchers = assetMatchers[os] || [];
  for (const matcher of matchers) {
    const asset = assets.find((item) => matcher.test(item.name || ''));
    if (asset?.browser_download_url) return asset;
  }
  return null;
};

const hydrateDownloadLinks = async () => {
  try {
    const response = await fetch(latestReleaseApi, { headers: { Accept: 'application/vnd.github+json' } });
    if (!response.ok) {
      setDownloadStatus('无法读取安装包列表，已保留 GitHub Release 下载页。', 'fallback');
      return;
    }
    const release = await response.json();
    const assets = Array.isArray(release.assets) ? release.assets : [];

    document.querySelectorAll('.download-option').forEach((option) => {
      const asset = chooseAsset(assets, option.dataset.os);
      if (!asset) return;
      option.href = asset.browser_download_url;
      option.querySelector('strong').textContent = asset.name;
    });

    const asset = chooseAsset(assets, currentOS);
    if (asset && primaryDownload) {
      primaryDownload.href = asset.browser_download_url;
      primaryDownload.textContent = `下载 ${osLabels[currentOS]} 版`;
      setDownloadStatus(`已匹配最新安装包：${asset.name}`, 'ready');
    } else {
      setDownloadStatus('没有找到当前系统的直接安装包，已保留 GitHub Release 下载页。', 'fallback');
    }
  } catch {
    setDownloadStatus('网络不可用或 GitHub API 被拦截，已保留 Release 下载页。', 'fallback');
  }
};

hydrateDownloadLinks();
