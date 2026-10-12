'use strict';

const encoder = new TextEncoder();
const decoder = new TextDecoder();
const status = document.querySelector('#status');
const video = document.querySelector('#video');
const authPanel = document.querySelector('#auth');
const passwordLabel = document.querySelector('#password-label');
const passwordInput = document.querySelector('#password');
const identityLabel = document.querySelector('#identity-label');
const identityInput = document.querySelector('#identity');
const authenticateButton = document.querySelector('#authenticate');

const signaling = new WebSocket(`${location.protocol === 'https:' ? 'wss:' : 'ws:'}//${location.host}/signal`);
signaling.binaryType = 'arraybuffer';

let selectedAuth;
let peer;
let control;
let realtime;
let remoteDescriptionSet = false;
const pendingCandidates = [];
const keysDown = new Set();
const buttonsDown = new Set();
const epoch = Date.now();
let controlSeq = 0;
let realtimeSeq = 0;
let totalDx = 0;
let totalDy = 0;
let pendingRealtime = null;

function concat(...arrays) {
  const size = arrays.reduce((sum, bytes) => sum + bytes.length, 0);
  const result = new Uint8Array(size);
  let offset = 0;
  for (const bytes of arrays) {
    result.set(bytes, offset);
    offset += bytes.length;
  }
  return result;
}

function encodeVarint(value) {
  let remaining = BigInt(value);
  const bytes = [];
  while (remaining > 0x7fn) {
    bytes.push(Number((remaining & 0x7fn) | 0x80n));
    remaining >>= 7n;
  }
  bytes.push(Number(remaining));
  return Uint8Array.from(bytes);
}

function fieldVarint(number, value) {
  return concat(encodeVarint((number << 3) | 0), encodeVarint(value));
}

function fieldBytes(number, value) {
  const bytes = typeof value === 'string' ? encoder.encode(value) : value;
  return concat(encodeVarint((number << 3) | 2), encodeVarint(bytes.length), bytes);
}

function clientHelloMessage() {
  const monitor = concat(
    fieldVarint(1, 0),
    fieldVarint(2, 0),
    fieldVarint(3, 0),
    fieldVarint(4, Math.max(1, screen.width)),
    fieldVarint(5, Math.max(1, screen.height)),
    fieldVarint(6, 60)
  );
  const hello = concat(
    fieldVarint(1, 1),
    fieldVarint(2, 6),
    fieldBytes(3, navigator.userAgent.slice(0, 256)),
    fieldBytes(4, monitor)
  );
  return fieldBytes(1, hello);
}

function readVarint(bytes, state) {
  let value = 0n;
  let shift = 0n;
  while (state.offset < bytes.length) {
    const byte = bytes[state.offset++];
    value |= BigInt(byte & 0x7f) << shift;
    if ((byte & 0x80) === 0) return Number(value);
    shift += 7n;
    if (shift > 63n) throw new Error('Malformed protobuf varint');
  }
  throw new Error('Truncated protobuf varint');
}

function parseFields(bytes) {
  const fields = new Map();
  const state = { offset: 0 };
  while (state.offset < bytes.length) {
    const tag = readVarint(bytes, state);
    const number = tag >>> 3;
    const wire = tag & 7;
    let value;
    if (wire === 0) {
      value = readVarint(bytes, state);
    } else if (wire === 2) {
      const length = readVarint(bytes, state);
      const end = state.offset + length;
      if (end > bytes.length) throw new Error('Truncated protobuf field');
      value = bytes.slice(state.offset, end);
      state.offset = end;
    } else if (wire === 1) {
      state.offset += 8;
      continue;
    } else if (wire === 5) {
      state.offset += 4;
      continue;
    } else {
      throw new Error(`Unsupported protobuf wire type ${wire}`);
    }
    if (state.offset > bytes.length) throw new Error('Truncated protobuf field');
    const values = fields.get(number) ?? [];
    values.push(value);
    fields.set(number, values);
  }
  return fields;
}

function first(fields, number) {
  return fields.get(number)?.[0];
}

function bytesToPem(der, label) {
  let binary = '';
  for (const byte of der) binary += String.fromCharCode(byte);
  const base64 = btoa(binary).match(/.{1,64}/g)?.join('\n') ?? '';
  return `-----BEGIN ${label}-----\n${base64}\n-----END ${label}-----`;
}

function pemBlock(text, label) {
  const begin = `-----BEGIN ${label}-----`;
  const end = `-----END ${label}-----`;
  const start = text.indexOf(begin);
  const finish = text.indexOf(end, start + begin.length);
  if (start < 0 || finish < 0) throw new Error(`Identity PEM missing ${label} block`);
  const encoded = text.slice(start + begin.length, finish).replace(/\s/g, '');
  const binary = atob(encoded);
  return Uint8Array.from(binary, character => character.charCodeAt(0));
}

function derLength(length) {
  if (length < 128) return Uint8Array.of(length);
  const parts = [];
  for (let value = length; value > 0; value >>>= 8) parts.unshift(value & 0xff);
  return Uint8Array.of(0x80 | parts.length, ...parts);
}

function der(tag, bytes) {
  return concat(Uint8Array.of(tag), derLength(bytes.length), bytes);
}

function rsaPkcs1PrivateToPkcs8(pkcs1) {
  const rsaEncryptionOid = Uint8Array.of(0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01);
  const version = Uint8Array.of(0x02, 0x01, 0x00);
  const algorithm = der(0x30, concat(der(0x06, rsaEncryptionOid), Uint8Array.of(0x05, 0x00)));
  return der(0x30, concat(version, algorithm, der(0x04, pkcs1)));
}

async function makeSignatureAuth(challenge) {
  const file = identityInput.files?.[0];
  if (!file) throw new Error('Choose a GSH RSA identity PEM file');
  const text = await file.text();
  const privateKeyDer = rsaPkcs1PrivateToPkcs8(pemBlock(text, 'RSA PRIVATE KEY'));
  const publicKeyPem = encoder.encode(bytesToPem(pemBlock(text, 'RSA PUBLIC KEY'), 'RSA PUBLIC KEY'));
  const privateKey = await crypto.subtle.importKey(
    'pkcs8',
    privateKeyDer,
    { name: 'RSASSA-PKCS1-v1_5', hash: 'SHA-256' },
    false,
    ['sign']
  );
  const signature = new Uint8Array(await crypto.subtle.sign(
    'RSASSA-PKCS1-v1_5', privateKey, challenge
  ));
  const signatureMessage = concat(fieldBytes(1, signature), fieldBytes(2, publicKeyPem));
  const authData = fieldBytes(2, signatureMessage);
  return fieldBytes(2, authData);
}

function makePasswordAuth(password) {
  const passwordMessage = fieldBytes(1, password);
  const authData = fieldBytes(1, passwordMessage);
  return fieldBytes(2, authData);
}

function clientMessageFromBinary(buffer) {
  const clientMessage = parseFields(new Uint8Array(buffer));
  const serverEvent = first(clientMessage, 1);
  if (!serverEvent) throw new Error('Server message has no event');
  return parseFields(serverEvent);
}

function setAuthMethod(serverHello) {
  const password = serverHello.has(4);
  const signatureBytes = first(serverHello, 5);
  if (!password && !signatureBytes) throw new Error('Server did not advertise an auth method');
  selectedAuth = password ? 'password' : 'signature';
  passwordLabel.hidden = !password;
  identityLabel.hidden = password;
  authPanel.hidden = false;
  status.textContent = password
    ? 'Enter server password to continue.'
    : 'Choose authorized GSH identity PEM to continue.';
  authenticateButton.onclick = async () => {
    authenticateButton.disabled = true;
    try {
      let authMessage;
      if (selectedAuth === 'password') {
        authMessage = makePasswordAuth(passwordInput.value);
      } else {
        const signatureMethod = parseFields(signatureBytes);
        const challenge = first(signatureMethod, 1);
        if (!(challenge instanceof Uint8Array)) throw new Error('Server signature challenge is missing');
        authMessage = await makeSignatureAuth(challenge);
      }
      signaling.send(authMessage);
      status.textContent = 'Authentication submitted; waiting for server verification.';
    } catch (error) {
      authenticateButton.disabled = false;
      status.textContent = `Authentication failed: ${error.message}`;
    }
  };
}

async function startWebRtc() {
  authPanel.hidden = true;
  peer = new RTCPeerConnection({ iceServers: [] });
  peer.addTransceiver('video', { direction: 'recvonly' });
  peer.addTransceiver('audio', { direction: 'recvonly' });
  control = peer.createDataChannel('gsh.control', { ordered: true });
  realtime = peer.createDataChannel('gsh.realtime', { ordered: false, maxRetransmits: 0 });
  realtime.bufferedAmountLowThreshold = 512;
  realtime.addEventListener('bufferedamountlow', flushRealtime);
  peer.ontrack = event => {
    if (event.track.kind === 'video') {
      video.srcObject = event.streams[0];
      status.textContent = 'Authenticated; receiving encrypted WebRTC desktop video.';
    }
  };
  peer.onicecandidate = event => {
    if (event.candidate) {
      signaling.send(JSON.stringify({ type: 'candidate', candidate: event.candidate.toJSON() }));
    }
  };
  const offer = await peer.createOffer();
  await peer.setLocalDescription(offer);
  signaling.send(JSON.stringify({ type: 'offer', sdp: offer.sdp }));
}

async function handleBinaryServerMessage(buffer) {
  const serverEvent = clientMessageFromBinary(buffer);
  if (serverEvent.has(1)) {
    const serverHello = parseFields(first(serverEvent, 1));
    setAuthMethod(serverHello);
  } else if (serverEvent.has(2)) {
    const ack = parseFields(first(serverEvent, 2));
    if ((first(ack, 1) ?? 0) !== 0) throw new Error(decoder.decode(first(ack, 2) ?? new Uint8Array()));
    passwordInput.value = '';
    identityInput.value = '';
    status.textContent = 'Authentication accepted; negotiating direct WebRTC connection…';
    await startWebRtc();
  } else if (serverEvent.has(3)) {
    status.textContent = `Server status: ${JSON.stringify(parseFields(first(serverEvent, 3)))}`;
  }
}

function sendControl(message) {
  if (control?.readyState !== 'open') return false;
  control.send(JSON.stringify({ ...message, seq: ++controlSeq, epoch }));
  return true;
}

function flushRealtime() {
  if (!realtime || realtime.readyState !== 'open' || !pendingRealtime) return;
  if (realtime.bufferedAmount > 2048) return;
  realtime.send(JSON.stringify(pendingRealtime));
  pendingRealtime = null;
}

function sendRealtime(message) {
  pendingRealtime = { ...message, seq: ++realtimeSeq, epoch };
  flushRealtime();
}

function position(event) {
  const bounds = video.getBoundingClientRect();
  return {
    x: Math.max(0, Math.min(1, (event.clientX - bounds.left) / bounds.width)),
    y: Math.max(0, Math.min(1, (event.clientY - bounds.top) / bounds.height))
  };
}

function releaseAll() {
  for (const code of keysDown) sendControl({ t: 'key', code, down: false });
  for (const button of buttonsDown) sendControl({ t: 'button', button, down: false, x: 0, y: 0 });
  keysDown.clear();
  buttonsDown.clear();
  sendControl({ t: 'release_all' });
}

signaling.onopen = () => {
  status.textContent = 'WSS connected; starting GSH client handshake.';
  signaling.send(clientHelloMessage());
};

signaling.onmessage = async event => {
  try {
    if (typeof event.data !== 'string') {
      await handleBinaryServerMessage(event.data);
      return;
    }
    const message = JSON.parse(event.data);
    if (message.type === 'answer') {
      await peer.setRemoteDescription({ type: 'answer', sdp: message.sdp });
      for (const candidate of pendingCandidates.splice(0)) await peer.addIceCandidate(candidate);
    } else if (message.type === 'candidate') {
      const candidate = message.candidate;
      if (peer.remoteDescription) await peer.addIceCandidate(candidate);
      else pendingCandidates.push(candidate);
    } else if (message.type === 'status') {
      status.textContent = message.message;
    }
  } catch (error) {
    status.textContent = `Connection failed: ${error.message}`;
    signaling.close();
  }
};

signaling.onerror = () => { status.textContent = 'WSS error; check TLS certificate and server log.'; };
signaling.onclose = releaseAll;

video.addEventListener('keydown', event => {
  if (keysDown.has(event.code)) return;
  keysDown.add(event.code);
  sendControl({ t: 'key', code: event.code, down: true });
  event.preventDefault();
});
video.addEventListener('keyup', event => {
  keysDown.delete(event.code);
  sendControl({ t: 'key', code: event.code, down: false });
  event.preventDefault();
});
video.addEventListener('pointermove', event => {
  if (document.pointerLockElement === video) {
    const bounds = video.getBoundingClientRect();
    const logicalWidth = video.videoWidth || 320;
    const logicalHeight = video.videoHeight || 240;
    totalDx += event.movementX * logicalWidth / Math.max(1, bounds.width);
    totalDy += event.movementY * logicalHeight / Math.max(1, bounds.height);
    sendRealtime({ t: 'relative_motion', dx_total: Math.round(totalDx), dy_total: Math.round(totalDy) });
  } else {
    sendRealtime({ t: 'pointer', ...position(event) });
  }
});
video.addEventListener('click', () => video.requestPointerLock?.());
video.addEventListener('pointerdown', event => {
  video.focus();
  video.setPointerCapture?.(event.pointerId);
  buttonsDown.add(event.button);
  sendControl({ t: 'button', button: event.button, down: true, ...position(event) });
  event.preventDefault();
});
video.addEventListener('pointerup', event => {
  buttonsDown.delete(event.button);
  sendControl({ t: 'button', button: event.button, down: false, ...position(event) });
  if (video.hasPointerCapture?.(event.pointerId)) video.releasePointerCapture(event.pointerId);
  event.preventDefault();
});
video.addEventListener('lostpointercapture', releaseAll);
video.addEventListener('wheel', event => {
  sendControl({ t: 'scroll', dx: event.deltaX, dy: event.deltaY, mode: event.deltaMode });
  event.preventDefault();
}, { passive: false });
window.addEventListener('blur', releaseAll);
document.addEventListener('visibilitychange', () => { if (document.hidden) releaseAll(); });
setInterval(() => {
  if (document.pointerLockElement === video) {
    sendRealtime({ t: 'relative_motion', dx_total: Math.round(totalDx), dy_total: Math.round(totalDy) });
  }
}, 50);
