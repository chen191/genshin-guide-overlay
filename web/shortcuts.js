const { invoke } = window.__TAURI__.core;

const elements = {
  play: document.querySelector('#play'),
  seekBackward: document.querySelector('#seek-backward'),
  seekForward: document.querySelector('#seek-forward'),
  visibility: document.querySelector('#visibility'),
  lock: document.querySelector('#lock'),
  save: document.querySelector('#save'),
  status: document.querySelector('#status'),
};

const hotkeyInputs = [elements.play, elements.seekBackward, elements.seekForward,
  elements.visibility, elements.lock];
let recording = null;
let valueBeforeRecording = '';

function errorText(error) {
  if (typeof error === 'string') return error;
  if (error && typeof error.message === 'string') return error.message;
  return '保存失败';
}

function showStatus(message, isError = false) {
  elements.status.textContent = message;
  elements.status.classList.toggle('error', isError);
}

function modifierNames(event) {
  const modifiers = [];
  if (event.ctrlKey) modifiers.push('Ctrl');
  if (event.altKey) modifiers.push('Alt');
  if (event.shiftKey) modifiers.push('Shift');
  if (event.metaKey) modifiers.push('Win');
  return modifiers;
}

function mainKeyName(code) {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  if (/^F(?:[1-9]|1[0-9]|2[0-4])$/.test(code)) return code;
  if (/^Numpad[0-9]$/.test(code)) return `Num${code.slice(6)}`;
  if (code.startsWith('Arrow')) return code.slice(5);

  const names = {
    Space: 'Space', Enter: 'Enter', Tab: 'Tab', Backspace: 'Backspace',
    Delete: 'Delete', Insert: 'Insert', Home: 'Home', End: 'End',
    PageUp: 'PageUp', PageDown: 'PageDown', PrintScreen: 'PrintScreen',
    ScrollLock: 'ScrollLock', Pause: 'Pause', CapsLock: 'CapsLock',
    Backquote: '`', Backslash: '\\', BracketLeft: '[', BracketRight: ']',
    Comma: ',', Equal: '=', Minus: '-', Period: '.', Quote: "'",
    Semicolon: ';', Slash: '/', NumpadAdd: 'NumAdd',
    NumpadDecimal: 'NumDecimal', NumpadDivide: 'NumDivide',
    NumpadEnter: 'NumEnter', NumpadEqual: 'NumEqual',
    NumpadMultiply: 'NumMultiply', NumpadSubtract: 'NumSubtract',
    AudioVolumeDown: 'AudioVolumeDown', AudioVolumeUp: 'AudioVolumeUp',
    AudioVolumeMute: 'AudioVolumeMute', MediaPlay: 'MediaPlay',
    MediaPause: 'MediaPause', MediaPlayPause: 'MediaPlayPause',
    MediaStop: 'MediaStop', MediaTrackNext: 'MediaTrackNext',
    MediaTrackPrevious: 'MediaTrackPrevious',
  };
  return names[code] || null;
}

function isModifier(code) {
  return ['ControlLeft', 'ControlRight', 'AltLeft', 'AltRight',
    'ShiftLeft', 'ShiftRight', 'MetaLeft', 'MetaRight'].includes(code);
}

function beginRecording(input) {
  if (recording && recording !== input) cancelRecording();
  if (recording === input) return;
  recording = input;
  valueBeforeRecording = input.value;
  input.value = '请按新的组合键';
  input.classList.add('recording');
  showStatus('按键录制中，Esc 取消');
}

function cancelRecording() {
  if (!recording) return;
  recording.value = valueBeforeRecording;
  recording.classList.remove('recording');
  recording = null;
  showStatus('');
}

function finishRecording(input, value) {
  input.value = value;
  input.classList.remove('recording');
  recording = null;
  showStatus(`已录制 ${value}`);
}

for (const input of hotkeyInputs) {
  input.addEventListener('focus', () => beginRecording(input));
  input.addEventListener('click', () => beginRecording(input));
  input.addEventListener('blur', () => {
    if (recording === input) cancelRecording();
  });
  input.addEventListener('keydown', (event) => {
    event.preventDefault();
    event.stopPropagation();
    if (event.repeat) return;
    if (recording !== input) beginRecording(input);
    if (event.code === 'Escape') {
      cancelRecording();
      input.blur();
      return;
    }
    if (isModifier(event.code)) {
      const preview = modifierNames(event);
      input.value = preview.length ? `${preview.join('+')}+…` : '请继续按主键';
      return;
    }
    const key = mainKeyName(event.code);
    if (!key) {
      showStatus('这个按键暂不支持，请换一个键', true);
      return;
    }
    finishRecording(input, [...modifierNames(event), key].join('+'));
  });
}

async function loadState() {
  try {
    const state = await invoke('get_overlay_state');
    elements.play.value = state.playHotkey;
    elements.seekBackward.value = state.seekBackwardHotkey;
    elements.seekForward.value = state.seekForwardHotkey;
    elements.visibility.value = state.visibilityHotkey;
    elements.lock.value = state.lockHotkey;
    if (state.hotkeyFailures?.length) {
      showStatus(`被占用：${state.hotkeyFailures.join('、')}`, true);
    }
  } catch (error) {
    showStatus(errorText(error), true);
  }
}

elements.save.addEventListener('click', async () => {
  if (recording) cancelRecording();
  elements.save.disabled = true;
  showStatus('正在保存…');
  try {
    await invoke('set_hotkeys', {
      play: elements.play.value,
      seekBackward: elements.seekBackward.value,
      seekForward: elements.seekForward.value,
      visibility: elements.visibility.value,
      lock: elements.lock.value,
    });
    showStatus('已保存并立即生效');
  } catch (error) {
    showStatus(errorText(error), true);
  } finally {
    elements.save.disabled = false;
  }
});

loadState();
