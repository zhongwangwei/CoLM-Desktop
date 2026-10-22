// 首帧就用记住的外观与语言（GUI 审查 S10）：样式表随模块脚本才加载、主题在 React 挂载后才设，
// 深色用户先看到一下浅色底，中文用户先看到英文 Loading…。CSP 只许同源脚本，所以放成独立文件，
// 紧跟在静态封面后面同步执行。
(function () {
  try {
    var root = document.documentElement;
    var dark = localStorage.getItem('colm-ui-theme') === 'dark';
    var en = localStorage.getItem('colm-ui-language') === 'en';
    if (dark) root.setAttribute('data-theme', 'dark');
    root.lang = en ? 'en' : 'zh-CN';
    root.style.colorScheme = dark ? 'dark' : 'light';
    root.style.backgroundColor = dark ? '#111e2e' : '#f6f8fa';
    var status = document.querySelector('.start-cover-status');
    if (status) status.textContent = en ? 'Loading…' : '正在加载…';
  } catch (error) {
    // 读不到存储（隐私模式等）就用默认的浅色中文封面。
  }
})();
