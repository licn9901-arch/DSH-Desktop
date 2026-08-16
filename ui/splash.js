"use strict";

const canvas = document.getElementById("digital-ocean");
const context = canvas.getContext("2d", { alpha: true });
const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");

let width = 0;
let height = 0;
let pixelRatio = 1;
let particles = [];
let animationFrame = 0;
let startedAt = performance.now();
let randomState = 0x4d534841;

/** 生成可重复的伪随机数，使每次启动的海面结构稳定。 */
function random() {
  randomState ^= randomState << 13;
  randomState ^= randomState >>> 17;
  randomState ^= randomState << 5;
  return (randomState >>> 0) / 4294967296;
}

/** 把数值限制在指定区间，避免动画阶段计算越界。 */
function clamp(value, minimum, maximum) {
  return Math.min(maximum, Math.max(minimum, value));
}

/** 使用平滑插值控制启动阶段的渐显节奏。 */
function smoothStep(minimum, maximum, value) {
  const progress = clamp((value - minimum) / (maximum - minimum), 0, 1);
  return progress * progress * (3 - 2 * progress);
}

/** 根据当前画布面积生成五层由两侧流向中央的数据粒子。 */
function createParticles() {
  randomState = 0x4d534841;
  const count = clamp(Math.round((width * height) / 1300), 560, 900);
  particles = Array.from({ length: count }, (_, index) => {
    const layer = index % 5;
    const brightnessRoll = random();
    return {
      layer,
      side: random() < 0.5 ? -1 : 1,
      progress: random(),
      speed: 0.011 + random() * 0.018,
      phase: random() * Math.PI * 2,
      offset: (random() + random() - 1) * (8 + layer * 3),
      size: brightnessRoll > 0.94 ? 1.35 + random() * 0.8 : 0.45 + random() * 0.75,
      opacity: brightnessRoll > 0.94 ? 0.58 : 0.16 + random() * 0.24,
      twinkleSpeed: 0.45 + random() * 1.1,
    };
  });
}

/** 同步高分屏画布尺寸，并在窗口变化后重建粒子密度。 */
function resizeCanvas() {
  const bounds = canvas.getBoundingClientRect();
  width = Math.max(1, bounds.width);
  height = Math.max(1, bounds.height);
  pixelRatio = Math.min(window.devicePixelRatio || 1, 1.5);
  canvas.width = Math.round(width * pixelRatio);
  canvas.height = Math.round(height * pixelRatio);
  context.setTransform(pixelRatio, 0, 0, pixelRatio, 0, 0);
  createParticles();
}

/** 计算指定层在某个横向位置与时刻的低幅波面高度。 */
function waveY(layer, normalizedX, seconds) {
  const base = height * (0.655 + layer * 0.049);
  const amplitude = 5 + layer * 1.8;
  const phase = layer * 1.17;
  const primary = Math.sin(normalizedX * 8.2 + seconds * (0.21 + layer * 0.018) + phase);
  const secondary = Math.sin(normalizedX * 17.5 - seconds * (0.12 + layer * 0.012) - phase) * 0.32;
  const convergence = Math.exp(-Math.pow((normalizedX - 0.5) / 0.2, 2)) * (7 - layer * 0.7);
  return base + (primary + secondary) * amplitude - convergence;
}

/** 绘制缓慢变化的中央蓝色光雾。 */
function drawFog(seconds, reveal) {
  const drift = Math.sin(seconds * 0.12) * width * 0.018;
  const centerX = width * 0.5 + drift;
  const centerY = height * 0.72;
  const gradient = context.createRadialGradient(centerX, centerY, 0, centerX, centerY, width * 0.48);
  gradient.addColorStop(0, `rgba(16, 95, 235, ${0.11 * reveal})`);
  gradient.addColorStop(0.42, `rgba(8, 45, 132, ${0.055 * reveal})`);
  gradient.addColorStop(1, "rgba(2, 6, 23, 0)");
  context.fillStyle = gradient;
  context.fillRect(0, height * 0.42, width, height * 0.58);
}

/** 绘制五条低透明度波面，给粒子海提供层次而不形成硬线条。 */
function drawWaveLayers(seconds, reveal) {
  for (let layer = 4; layer >= 0; layer -= 1) {
    context.beginPath();
    for (let step = 0; step <= 180; step += 1) {
      const normalizedX = step / 180;
      const x = normalizedX * width;
      const y = waveY(layer, normalizedX, seconds);
      if (step === 0) context.moveTo(x, y);
      else context.lineTo(x, y);
    }
    context.strokeStyle = `rgba(31, ${112 + layer * 12}, 255, ${(0.035 + layer * 0.006) * reveal})`;
    context.lineWidth = 0.7;
    context.stroke();
  }
}

/** 绘制向中央汇聚、带独立闪烁相位的主粒子与微粒。 */
function drawParticles(seconds, reveal) {
  for (const particle of particles) {
    const progress = (particle.progress + seconds * particle.speed) % 1;
    const normalizedX = particle.side < 0
      ? -0.025 + progress * 0.525
      : 1.025 - progress * 0.525;
    const x = normalizedX * width;
    const y = waveY(particle.layer, normalizedX, seconds) + particle.offset;
    const edgeFade = smoothStep(0, 0.12, progress);
    const sinkFade = 1 - smoothStep(0.91, 1, progress);
    const centerDensity = 0.68 + progress * 0.5;
    const twinkle = 0.7 + Math.sin(seconds * particle.twinkleSpeed + particle.phase) * 0.3;
    const alpha = particle.opacity * edgeFade * sinkFade * centerDensity * twinkle * reveal;

    context.beginPath();
    context.arc(x, y, particle.size, 0, Math.PI * 2);
    context.fillStyle = `rgba(${particle.layer < 2 ? 76 : 38}, ${148 + particle.layer * 13}, 255, ${alpha})`;
    context.fill();
  }
}

/** 每 3.6 秒从中央向两侧扩散一层极淡光波。 */
function drawExpansionWave(seconds, reveal) {
  const phase = (seconds % 3.6) / 3.6;
  const alpha = Math.sin(phase * Math.PI) * 0.1 * reveal;
  const radiusX = width * (0.08 + phase * 0.48);
  const radiusY = 4 + phase * 26;
  const coreY = height * 0.648;
  context.beginPath();
  context.ellipse(width * 0.5, coreY, radiusX, radiusY, 0, Math.PI, Math.PI * 2);
  context.strokeStyle = `rgba(72, 157, 255, ${alpha})`;
  context.lineWidth = 1;
  context.stroke();
}

/** 绘制远处能量核心的亮度与光晕呼吸。 */
function drawCore(seconds, reveal) {
  const breath = (Math.sin((seconds / 2.8) * Math.PI * 2 - Math.PI / 2) + 1) / 2;
  const radius = 24 + breath * 56;
  const opacity = (0.2 + breath * 0.8) * reveal;
  const coreX = width * 0.5;
  const coreY = height * 0.648;
  const glow = context.createRadialGradient(coreX, coreY, 0, coreX, coreY, radius);
  glow.addColorStop(0, `rgba(201, 241, 255, ${0.92 * opacity})`);
  glow.addColorStop(0.08, `rgba(62, 180, 255, ${0.72 * opacity})`);
  glow.addColorStop(0.34, `rgba(18, 102, 255, ${0.26 * opacity})`);
  glow.addColorStop(1, "rgba(9, 54, 182, 0)");
  context.fillStyle = glow;
  context.fillRect(coreX - radius, coreY - radius, radius * 2, radius * 2);
}

/** 绘制启动序列的一帧，并在可见状态持续循环。 */
function drawFrame(timestamp) {
  const seconds = (timestamp - startedAt) / 1000;
  const oceanReveal = smoothStep(0.5, 1.5, seconds);
  const coreReveal = smoothStep(1.5, 2.5, seconds);
  context.clearRect(0, 0, width, height);
  drawFog(seconds, oceanReveal);
  drawExpansionWave(seconds, coreReveal);
  drawWaveLayers(seconds, oceanReveal);
  drawParticles(seconds, oceanReveal);
  drawCore(seconds, coreReveal);

  if (!reducedMotion.matches && !document.hidden) {
    animationFrame = requestAnimationFrame(drawFrame);
  }
}

/** 在窗口重新可见时恢复动画，隐藏时停止消耗绘制资源。 */
function updateVisibility() {
  cancelAnimationFrame(animationFrame);
  if (!document.hidden) {
    animationFrame = requestAnimationFrame(drawFrame);
  }
}

resizeCanvas();
window.addEventListener("resize", resizeCanvas);
document.addEventListener("visibilitychange", updateVisibility);

if (reducedMotion.matches) {
  drawFrame(startedAt + 4000);
} else {
  animationFrame = requestAnimationFrame(drawFrame);
}
