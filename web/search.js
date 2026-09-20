const state = document.querySelector('#state');
const results = document.querySelector('#results');
const keyword = new URLSearchParams(location.search).get('keyword')?.trim() || '';

function setError(message) {
  state.textContent = message;
  state.classList.add('error');
  state.hidden = false;
  results.hidden = true;
}

function plainText(value) {
  const documentValue = new DOMParser().parseFromString(String(value || ''), 'text/html');
  return documentValue.body.textContent?.trim() || '';
}

function formatCount(value) {
  const count = Number(value) || 0;
  if (count >= 10000) return `${(count / 10000).toFixed(count >= 100000 ? 0 : 1)}万`;
  return String(count);
}

function addText(parent, className, text) {
  const element = document.createElement('span');
  element.className = className;
  element.textContent = text;
  parent.append(element);
}

function renderCard(video) {
  const link = document.createElement('a');
  link.className = 'card';
  link.href = `https://www.bilibili.com/video/${encodeURIComponent(video.bvid)}`;
  link.target = '_blank';
  link.rel = 'noopener';

  const thumbWrap = document.createElement('div');
  thumbWrap.className = 'thumb-wrap';
  const thumb = document.createElement('img');
  thumb.className = 'thumb';
  thumb.loading = 'lazy';
  thumb.referrerPolicy = 'no-referrer';
  thumb.alt = '';
  const imageUrl = String(video.pic || '');
  thumb.src = imageUrl.startsWith('//') ? `https:${imageUrl}` : imageUrl;
  thumbWrap.append(thumb);
  if (video.duration) addText(thumbWrap, 'duration', String(video.duration));

  const info = document.createElement('div');
  info.className = 'info';
  addText(info, 'title', plainText(video.title) || '未命名视频');
  const meta = document.createElement('div');
  meta.className = 'meta';
  addText(meta, 'author', video.author || '未知UP主');
  addText(meta, 'play', `播放 ${formatCount(video.play)}`);
  info.append(meta);

  link.append(thumbWrap, info);
  return link;
}

window.renderSearchResults = (payload) => {
  const videos = Array.isArray(payload?.data?.result)
    ? payload.data.result.filter((video) => /^BV1[0-9A-Za-z]{9}$/.test(video?.bvid || ''))
    : [];
  if (payload?.code !== 0) {
    setError(payload?.message ? `搜索失败：${payload.message}` : '搜索暂时不可用');
    return;
  }
  if (!videos.length) {
    setError('没有找到相关视频，请换个关键词');
    return;
  }
  const fragment = document.createDocumentFragment();
  videos.forEach((video) => fragment.append(renderCard(video)));
  results.replaceChildren(fragment);
  state.hidden = true;
  results.hidden = false;
};

if (!keyword) {
  setError('搜索关键词为空');
} else {
  document.title = `${keyword} - B站攻略搜索`;
  state.textContent = `正在搜索“${keyword}”…`;
}
