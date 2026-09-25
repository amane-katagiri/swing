export const DOWNSCALE_STEPS = [1920, 1280, 1024, 800, 640, 512, 400, 320, 240];
export const STORAGE_TARGET_CHARS = 2.5 * 1024 * 1024;
export const DEFAULT_COLOR = '#008080';

export function quantize5(v) {
  const level = Math.round((v / 255) * 31);
  return (level << 3) | (level >> 2);
}

export function quantize6(v) {
  const level = Math.round((v / 255) * 63);
  return (level << 2) | (level >> 4);
}

export function quantizeColorHex(hex) {
  const m = /^#?([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(hex || '');
  if (!m) return DEFAULT_COLOR;
  const r = quantize5(parseInt(m[1], 16));
  const g = quantize6(parseInt(m[2], 16));
  const b = quantize5(parseInt(m[3], 16));
  return `#${[r, g, b].map((v) => v.toString(16).padStart(2, '0')).join('')}`;
}

export function decodeImageFile(file) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error('ファイルを読み込めませんでした'));
    reader.onload = () => {
      const img = new Image();
      img.onerror = () => reject(new Error('画像を解釈できませんでした'));
      img.onload = () => resolve(img);
      img.src = reader.result;
    };
    reader.readAsDataURL(file);
  });
}

/* Not composited with the background color so a later color change still shows through. */
export function quantizeCanvas(ctx, w, h) {
  const imgData = ctx.getImageData(0, 0, w, h);
  const d = imgData.data;
  for (let i = 0; i < d.length; i += 4) {
    const a = d[i + 3] >= 128 ? 255 : 0;
    if (a === 0) {
      d[i] = 0;
      d[i + 1] = 0;
      d[i + 2] = 0;
    } else {
      d[i] = quantize5(d[i]);
      d[i + 1] = quantize6(d[i + 1]);
      d[i + 2] = quantize5(d[i + 2]);
    }
    d[i + 3] = a;
  }
  ctx.putImageData(imgData, 0, 0);
}

/* One large drawImage() reduction aliases even with imageSmoothingQuality "high". */
export function drawScaledSmooth(ctx, img, dw, dh) {
  let src = img;
  let srcW = img.naturalWidth || img.width;
  let srcH = img.naturalHeight || img.height;
  while (srcW / 2 > dw && srcH / 2 > dh) {
    const nextW = Math.max(dw, Math.round(srcW / 2));
    const nextH = Math.max(dh, Math.round(srcH / 2));
    const step = document.createElement('canvas');
    step.width = nextW;
    step.height = nextH;
    const stepCtx = step.getContext('2d');
    stepCtx.imageSmoothingEnabled = true;
    stepCtx.imageSmoothingQuality = 'high';
    stepCtx.drawImage(src, 0, 0, srcW, srcH, 0, 0, nextW, nextH);
    src = step;
    srcW = nextW;
    srcH = nextH;
  }
  ctx.imageSmoothingEnabled = true;
  ctx.imageSmoothingQuality = 'high';
  ctx.drawImage(src, 0, 0, srcW, srcH, 0, 0, dw, dh);
}

export function renderQuantizedPng(img, maxLong) {
  const natW = img.naturalWidth || img.width;
  const natH = img.naturalHeight || img.height;
  const scale = Math.min(1, maxLong / Math.max(natW, natH));
  const w = Math.max(1, Math.round(natW * scale));
  const h = Math.max(1, Math.round(natH * scale));
  const canvas = document.createElement('canvas');
  canvas.width = w;
  canvas.height = h;
  const ctx = canvas.getContext('2d');
  drawScaledSmooth(ctx, img, w, h);
  quantizeCanvas(ctx, w, h);
  return { dataUrl: canvas.toDataURL('image/png'), width: w, height: h };
}

export async function processImageFile(file) {
  const img = await decodeImageFile(file);
  const natMax = Math.max(img.naturalWidth || img.width, img.naturalHeight || img.height);
  const candidates = [natMax, ...DOWNSCALE_STEPS.filter((s) => s < natMax)];
  let result = null;
  for (const maxLong of candidates) {
    result = renderQuantizedPng(img, maxLong);
    if (result.dataUrl.length <= STORAGE_TARGET_CHARS) break;
  }
  return result;
}
