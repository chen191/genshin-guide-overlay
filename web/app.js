const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const overlayWindow = window.__TAURI__.window.getCurrentWindow();

const elements = {
  url: document.querySelector('#url'),
  open: document.querySelector('#open'),
  backSearch: document.querySelector('#back-search'),
  quality: document.querySelector('#quality'),
  speed: document.querySelector('#speed'),
  reload: document.querySelector('#reload'),
  zoom: document.querySelector('#zoom'),
  seekBack: document.querySelector('#seek-back'),
  play: document.querySelector('#play'),
  seekForward: document.querySelector('#seek-forward'),
  previousEpisode: document.querySelector('#previous-episode'),
  nextEpisode: document.querySelector('#next-episode'),
  shortcuts: document.querySelector('#shortcuts'),
  opacity: document.querySelector('#opacity'),
  lock: document.querySelector('#lock'),
  hide: document.querySelector('#hide'),
  drag: document.querySelector('#drag'),
  resize: document.querySelector('#resize'),
  status: document.querySelector('#status'),
};

let statusTimer = 0;
let lastKnownPlaybackRate = 1;
let currentZoom = 40;
let currentVideoOnly = false;
let currentSearchResults = false;
const zoomLevels = [40, 50, 60, 75, 90, 100];

function errorText(error) {
  if (typeof error === 'string') return error;
  if (error && typeof error.message === 'string') return error.message;
  return '操作失败';
}

function showStatus(message, isError = false, duration = 1800) {
  window.clearTimeout(statusTimer);
  elements.status.textContent = message;
  elements.status.classList.toggle('error', isError);
  elements.status.classList.add('visible');
  statusTimer = window.setTimeout(() => elements.status.classList.remove('visible'), duration);
}

async function call(command, args = {}) {
  try {
    return await invoke(command, args);
  } catch (error) {
    showStatus(errorText(error), true, 2600);
    throw error;
  }
}

function applyState(state) {
  if (!state) return;
  if (state.url && document.activeElement !== elements.url) elements.url.value = state.url;
  elements.opacity.value = String(state.opacity);
  currentZoom = state.zoom;
  currentVideoOnly = Boolean(state.videoOnly);
  currentSearchResults = Boolean(state.searchResults);
  elements.zoom.textContent = `${state.zoom}%`;
  elements.zoom.hidden = currentVideoOnly || currentSearchResults;
  elements.backSearch.hidden = currentSearchResults || !state.lastSearchInput;
  elements.quality.hidden = !currentVideoOnly;
  elements.speed.hidden = !currentVideoOnly;
  elements.seekBack.hidden = !currentVideoOnly;
  elements.seekForward.hidden = !currentVideoOnly;
  elements.previousEpisode.hidden = !currentVideoOnly;
  elements.nextEpisode.hidden = !currentVideoOnly;
  if (Number.isFinite(state.playbackRate)) {
    const playbackRate = String(state.playbackRate);
    if (elements.speed.querySelector(`option[value="${playbackRate}"]`)) {
      elements.speed.value = playbackRate;
      lastKnownPlaybackRate = state.playbackRate;
    }
  }
  elements.play.textContent = '⏯';
  elements.play.title = `暂停/继续（${state.playHotkey}）`;
  elements.seekBack.title = `快退 10 秒（${state.seekBackwardHotkey}）`;
  elements.seekForward.title = `快进 10 秒（${state.seekForwardHotkey}）`;
  elements.lock.textContent = state.locked ? '解锁' : '锁定';
  elements.lock.title = state.locked
    ? `${state.lockHotkey} 解锁鼠标穿透`
    : `${state.lockHotkey} 锁定鼠标穿透`;
  elements.hide.title = `隐藏到托盘（${state.visibilityHotkey} 恢复）`;
  if (state.hotkeyFailures?.length) {
    showStatus(`快捷键被占用：${state.hotkeyFailures.join('、')}`, true, 4200);
  }
}

async function openUrl() {
  const value = elements.url.value.trim();
  if (!value) {
    showStatus('请输入攻略关键词或网址', true);
    return;
  }
  showStatus('正在搜索 / 打开…', false, 12000);
  try {
    await call('navigate_content', { input: value });
    showStatus('已打开');
  } catch (_) {
    // call() already surfaced the backend error in the toolbar.
  }
}

elements.open.addEventListener('click', openUrl);
elements.backSearch.addEventListener('click', async () => {
  await call('return_to_search');
  showStatus('已返回上一层搜索');
});
elements.quality.addEventListener('click', () => call('show_quality'));
elements.speed.addEventListener('change', async () => {
  const requested = Number(elements.speed.value);
  try {
    const applied = await call('set_playback_rate', { value: requested });
    elements.speed.value = String(applied);
    lastKnownPlaybackRate = applied;
  } catch (_) {
    elements.speed.value = String(lastKnownPlaybackRate);
  }
});
elements.url.addEventListener('keydown', (event) => {
  if (event.key === 'Enter') openUrl();
});
elements.reload.addEventListener('click', () => call('content_reload'));
elements.zoom.addEventListener('click', async () => {
  const index = zoomLevels.findIndex((value) => value > currentZoom);
  const requested = index >= 0 ? zoomLevels[index] : zoomLevels[0];
  const applied = await call('set_content_zoom', { value: requested });
  currentZoom = applied;
  elements.zoom.textContent = `${applied}%`;
  showStatus(`攻略页缩放 ${applied}%`);
});
elements.play.addEventListener('click', () => call('toggle_media'));
elements.seekBack.addEventListener('click', () => call('seek_backward'));
elements.seekForward.addEventListener('click', () => call('seek_forward'));

const episodeButtons = [elements.previousEpisode, elements.nextEpisode];

async function navigateEpisode(command, loadingMessage, fallbackMessage) {
  if (episodeButtons.some((button) => button.disabled)) return;
  episodeButtons.forEach((button) => { button.disabled = true; });
  showStatus(loadingMessage, false, 12000);
  try {
    const message = await call(command);
    showStatus(message || fallbackMessage);
  } catch (_) {
    // call() already surfaced the backend error in the toolbar.
  } finally {
    episodeButtons.forEach((button) => { button.disabled = false; });
  }
}

elements.previousEpisode.addEventListener('click', () => {
  navigateEpisode('previous_episode', '正在查找上一集…', '已打开上一集');
});
elements.nextEpisode.addEventListener('click', () => {
  navigateEpisode('next_episode', '正在查找下一集…', '已打开下一集');
});
elements.shortcuts.addEventListener('click', () => call('open_shortcut_settings'));
elements.lock.addEventListener('click', () => call('toggle_click_through'));
elements.hide.addEventListener('click', () => call('hide_overlay'));

elements.drag.addEventListener('pointerdown', (event) => {
  if (event.button === 0) overlayWindow.startDragging().catch((error) => showStatus(errorText(error), true));
});
elements.resize.addEventListener('pointerdown', (event) => {
  if (event.button === 0) overlayWindow.startResizeDragging('SouthEast').catch((error) => showStatus(errorText(error), true));
});

elements.opacity.addEventListener('change', () => {
  call('set_opacity', { value: Number(elements.opacity.value) });
});

listen('overlay-state', (event) => applyState(event.payload));
listen('navigation-state', (event) => {
  const payload = event.payload;
  if (payload?.url && document.activeElement !== elements.url) elements.url.value = payload.url;
});
listen('media-state', (event) => {
  const payload = event.payload;
  showStatus(payload?.message || '无法确认视频状态', !payload?.ok, payload?.ok ? 1600 : 2600);
});
listen('quality-state', (event) => {
  const payload = event.payload;
  showStatus(payload?.message || '无法确认当前画质', !payload?.ok, 4200);
});
listen('speed-state', (event) => {
  const payload = event.payload;
  if (payload?.ok && Number.isFinite(payload.rate)) {
    elements.speed.value = String(payload.rate);
    lastKnownPlaybackRate = payload.rate;
  }
  showStatus(payload?.message || '无法确认当前播放速度', !payload?.ok, 2400);
});
listen('settings-error', (event) => {
  showStatus(event.payload || '设置保存失败', true, 5000);
});

call('get_overlay_state').then(applyState);
