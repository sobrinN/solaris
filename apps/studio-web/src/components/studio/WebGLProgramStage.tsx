import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Sparkles } from "lucide-react";
import type {
  BrandTokens,
  CaptionBlock,
  Clip,
  ColorGrade,
  MediaAsset,
  ParsedCubeLut,
  SolarisEngineClient,
} from "@solaris/engine-client";
import { buildSandboxedGfxDoc, escapeHtml } from "@solaris/gfx-runtime";
import type { SampledPixelFrame } from "./BroadcastScopes";
import { SandboxedGfxFrame } from "./SandboxedGfxFrame";

export type { SampledPixelFrame };

export interface StageVisualLayer {
  trackId: "v1" | "v2";
  clip: Clip;
  media: MediaAsset;
  url: string | null;
}

interface WebGLProgramStageProps {
  client?: SolarisEngineClient;
  ticketVersion?: number;
  width: number;
  height: number;
  playheadSec: number;
  isPlaying: boolean;
  grade: ColorGrade;
  activeLayers: StageVisualLayer[];
  activeGfxClips: Clip[];
  activeCaptionClip: Clip | null;
  brandTokens: BrandTokens;
  brandFontCss?: string;
  brandLut: ParsedCubeLut | null;
  onPixelFrame?: (frame: SampledPixelFrame | null) => void;
}

const VERT_SRC = `#version 300 es
in vec2 a_pos;
out vec2 v_uv;
void main() {
  v_uv = a_pos * 0.5 + 0.5;
  v_uv.y = 1.0 - v_uv.y;
  gl_Position = vec4(a_pos, 0.0, 1.0);
}`;

const FRAG_SRC = `#version 300 es
precision highp float;
precision highp sampler3D;

in vec2 v_uv;
out vec4 outColor;

uniform sampler2D u_tex;
uniform sampler3D u_lut;
uniform float u_hasLut;
uniform float u_lutSize;
uniform float u_lutIntensity;
uniform vec3 u_lutDomainMin;
uniform vec3 u_lutDomainMax;
uniform vec3 u_lift;
uniform vec3 u_gamma;
uniform vec3 u_gain;
uniform float u_temp;
uniform float u_tint;
uniform float u_contrast;
uniform float u_sat;
uniform float u_exp;
uniform vec2 u_translate;
uniform float u_scale;
uniform float u_rotation;
uniform float u_opacity;
uniform vec4 u_reframe;
uniform float u_srcAspect;
uniform float u_dstAspect;

void main() {
  // Apply clip translation (normalized canvas offset), rotation, and scale
  vec2 p = v_uv - 0.5 - u_translate;
  // Convert to aspect-corrected space for distortion-free rotation
  p.x *= u_dstAspect;
  float c = cos(-u_rotation);
  float s = sin(-u_rotation);
  p = mat2(c, -s, s, c) * p;
  p.x /= max(0.01, u_dstAspect);

  // Scale transform
  p /= max(0.05, u_scale);

  // Cover-fit effective reframed source aspect into destination canvas aspect
  float effSrcAspect = u_srcAspect * (u_reframe.z / max(0.01, u_reframe.w));
  if (effSrcAspect > u_dstAspect) {
    p.x *= u_dstAspect / max(0.01, effSrcAspect);
  } else {
    p.y *= effSrcAspect / max(0.01, u_dstAspect);
  }

  vec2 normUv = p + 0.5;
  if (normUv.x < 0.0 || normUv.x > 1.0 || normUv.y < 0.0 || normUv.y > 1.0) {
    discard;
  }

  // Map into smart_reframe sub-box
  vec2 sampleUv = u_reframe.xy + normUv * u_reframe.zw;
  if (sampleUv.x < 0.0 || sampleUv.x > 1.0 || sampleUv.y < 0.0 || sampleUv.y > 1.0) {
    discard;
  }

  vec4 texel = texture(u_tex, sampleUv);
  vec3 col = texel.rgb;

  // Lift / Gamma / Gain + Temp/Tint
  col = clamp(col + u_lift * (1.0 - col) + u_gain * col, 0.0, 1.0);
  col.r += u_temp * 0.14;
  col.b -= u_temp * 0.14;
  col.g += u_tint * 0.12;
  col = clamp(col, 0.0, 1.0);
  vec3 gammaPow = max(vec3(0.35), 1.0 - u_gamma * 0.65);
  col = pow(col, gammaPow);

  // Exposure, Contrast & Saturation
  col *= pow(2.0, u_exp);
  col = (col - 0.5) * u_contrast + 0.5;
  float luma = dot(col, vec3(0.2126, 0.7152, 0.0722));
  col = mix(vec3(luma), col, u_sat);
  col = clamp(col, 0.0, 1.0);

  if (u_hasLut > 0.5 && u_lutIntensity > 0.001 && u_lutSize >= 2.0) {
    vec3 denom = max(vec3(0.0001), u_lutDomainMax - u_lutDomainMin);
    vec3 lutCoord = clamp((col - u_lutDomainMin) / denom, 0.0, 1.0);
    // Half-texel offset per OpenGL 3D LUT specification so [0,1] maps to exact boundary texel centers
    lutCoord = (lutCoord * (u_lutSize - 1.0) + 0.5) / u_lutSize;
    vec3 lutCol = texture(u_lut, lutCoord).rgb;
    col = mix(col, lutCol, clamp(u_lutIntensity, 0.0, 1.0));
  }

  outColor = vec4(col, texel.a * clamp(u_opacity, 0.0, 1.0));
}`;

interface CachedGL {
  gl: WebGL2RenderingContext;
  prog: WebGLProgram;
  tex: WebGLTexture;
  lutTex: WebGLTexture;
  hasLut: boolean;
  lutSize: number;
  domainMin: [number, number, number];
  domainMax: [number, number, number];
  uniforms: Record<string, WebGLUniformLocation | null>;
}

export function WebGLProgramStage({
  client,
  ticketVersion = 0,
  width,
  height,
  playheadSec,
  isPlaying,
  grade,
  activeLayers,
  activeGfxClips,
  activeCaptionClip,
  brandTokens,
  brandFontCss = "",
  brandLut,
  onPixelFrame,
}: WebGLProgramStageProps) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const videoRefs = useRef<Record<string, HTMLVideoElement | null>>({});
  const pendingVideoSeekRef = useRef<Record<string, number>>({});
  const imgRefs = useRef<Record<string, HTMLImageElement | null>>({});
  const iframeRefs = useRef<Record<string, HTMLIFrameElement | null>>({});
  const glStateRef = useRef<CachedGL | null>(null);
  const sampleCanvasRef = useRef<HTMLCanvasElement | null>(null);
  const lastSampleTimeRef = useRef<number>(0);
  const playheadRef = useRef<number>(playheadSec);
  playheadRef.current = playheadSec;
  const activeGfxRef = useRef<Clip[]>(activeGfxClips);
  activeGfxRef.current = activeGfxClips;
  const [glRestoreSeq, setGlRestoreSeq] = useState(0);

  const playableLayers = useMemo(
    () =>
      activeLayers.filter(
        (layer) =>
          Boolean(layer.url) &&
          (layer.media.kind === "video" || layer.media.kind === "image"),
      ),
    [activeLayers],
  );
  const hasVisualMedia = playableLayers.length > 0;

  const playableLayersRef = useRef<StageVisualLayer[]>(playableLayers);
  playableLayersRef.current = playableLayers;
  const gradeRef = useRef<ColorGrade>(grade);
  gradeRef.current = grade;
  const onPixelFrameRef = useRef<typeof onPixelFrame>(onPixelFrame);
  onPixelFrameRef.current = onPixelFrame;

  // Handle WebGL context loss / restoration cleanly
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const onLost = (e: Event) => {
      e.preventDefault();
      glStateRef.current = null;
    };
    const onRestored = () => {
      setGlRestoreSeq((s) => s + 1);
    };
    canvas.addEventListener("webglcontextlost", onLost);
    canvas.addEventListener("webglcontextrestored", onRestored);
    return () => {
      canvas.removeEventListener("webglcontextlost", onLost);
      canvas.removeEventListener("webglcontextrestored", onRestored);
    };
  }, []);

  // Initialize WebGL2 pipeline once (and re-init on context restore)
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const gl = canvas.getContext("webgl2", {
      preserveDrawingBuffer: true,
      alpha: true,
    });
    if (!gl) return;

    const vs = gl.createShader(gl.VERTEX_SHADER);
    const fs = gl.createShader(gl.FRAGMENT_SHADER);
    const prog = gl.createProgram();
    if (!vs || !fs || !prog) return;

    gl.shaderSource(vs, VERT_SRC);
    gl.compileShader(vs);
    if (!gl.getShaderParameter(vs, gl.COMPILE_STATUS)) {
      gl.deleteShader(vs);
      gl.deleteShader(fs);
      gl.deleteProgram(prog);
      return;
    }

    gl.shaderSource(fs, FRAG_SRC);
    gl.compileShader(fs);
    if (!gl.getShaderParameter(fs, gl.COMPILE_STATUS)) {
      gl.deleteShader(vs);
      gl.deleteShader(fs);
      gl.deleteProgram(prog);
      return;
    }

    gl.attachShader(prog, vs);
    gl.attachShader(prog, fs);
    gl.linkProgram(prog);
    if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) {
      gl.deleteShader(vs);
      gl.deleteShader(fs);
      gl.deleteProgram(prog);
      return;
    }

    const buf = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, buf);
    gl.bufferData(
      gl.ARRAY_BUFFER,
      new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]),
      gl.STATIC_DRAW,
    );
    const loc = gl.getAttribLocation(prog, "a_pos");
    gl.enableVertexAttribArray(loc);
    gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);

    const tex = gl.createTexture()!;
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);

    const lutTex = gl.createTexture()!;
    gl.bindTexture(gl.TEXTURE_3D, lutTex);
    gl.texParameteri(gl.TEXTURE_3D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_3D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_3D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_3D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_3D, gl.TEXTURE_WRAP_R, gl.CLAMP_TO_EDGE);
    gl.texImage3D(
      gl.TEXTURE_3D,
      0,
      gl.RGBA,
      1,
      1,
      1,
      0,
      gl.RGBA,
      gl.UNSIGNED_BYTE,
      new Uint8Array([255, 255, 255, 255]),
    );

    const uniformNames = [
      "u_tex",
      "u_lut",
      "u_hasLut",
      "u_lutSize",
      "u_lutIntensity",
      "u_lutDomainMin",
      "u_lutDomainMax",
      "u_lift",
      "u_gamma",
      "u_gain",
      "u_temp",
      "u_tint",
      "u_contrast",
      "u_sat",
      "u_exp",
      "u_translate",
      "u_scale",
      "u_rotation",
      "u_opacity",
      "u_reframe",
      "u_srcAspect",
      "u_dstAspect",
    ];
    const uniforms: Record<string, WebGLUniformLocation | null> = {};
    for (const name of uniformNames) {
      uniforms[name] = gl.getUniformLocation(prog, name);
    }

    glStateRef.current = {
      gl,
      prog,
      tex,
      lutTex,
      hasLut: false,
      lutSize: 1,
      domainMin: [0, 0, 0],
      domainMax: [1, 1, 1],
      uniforms,
    };

    return () => {
      if (buf) gl.deleteBuffer(buf);
      gl.deleteProgram(prog);
      gl.deleteShader(vs);
      gl.deleteShader(fs);
      gl.deleteTexture(tex);
      gl.deleteTexture(lutTex);
      glStateRef.current = null;
    };
  }, [glRestoreSeq]);

  const drawCurrentFrame = useCallback(() => {
    const state = glStateRef.current;
    const canvas = canvasRef.current;
    if (!state || !canvas) return;
    const { gl, prog, tex, lutTex, hasLut, lutSize, domainMin, domainMax, uniforms } =
      state;

    gl.viewport(0, 0, canvas.width, canvas.height);
    gl.clearColor(0.031, 0.016, 0.016, 1.0);
    gl.clear(gl.COLOR_BUFFER_BIT);

    const currentLayers = playableLayersRef.current;
    if (currentLayers.length === 0) return;

    const currentGrade = gradeRef.current;

    gl.enable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
    gl.useProgram(prog);

    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_3D, lutTex);

    gl.uniform1i(uniforms.u_tex, 0);
    gl.uniform1i(uniforms.u_lut, 1);
    gl.uniform1f(uniforms.u_hasLut, hasLut ? 1.0 : 0.0);
    gl.uniform1f(uniforms.u_lutSize, lutSize);
    gl.uniform1f(uniforms.u_lutIntensity, currentGrade.lut_intensity);
    gl.uniform3f(uniforms.u_lutDomainMin, domainMin[0], domainMin[1], domainMin[2]);
    gl.uniform3f(uniforms.u_lutDomainMax, domainMax[0], domainMax[1], domainMax[2]);
    gl.uniform3f(uniforms.u_lift, currentGrade.lift.r, currentGrade.lift.g, currentGrade.lift.b);
    gl.uniform3f(uniforms.u_gamma, currentGrade.gamma.r, currentGrade.gamma.g, currentGrade.gamma.b);
    gl.uniform3f(uniforms.u_gain, currentGrade.gain.r, currentGrade.gain.g, currentGrade.gain.b);
    gl.uniform1f(uniforms.u_temp, currentGrade.temp);
    gl.uniform1f(uniforms.u_tint, currentGrade.tint);
    gl.uniform1f(uniforms.u_contrast, currentGrade.contrast);
    gl.uniform1f(uniforms.u_sat, currentGrade.saturation);
    gl.uniform1f(uniforms.u_exp, currentGrade.exposure);
    gl.uniform1f(uniforms.u_dstAspect, width / Math.max(1, height));

    let drewAny = false;
    const nowSec = playheadRef.current;

    for (const layer of currentLayers) {
      const key = `${layer.trackId}:${layer.clip.id}`;
      let source: TexImageSource | null = null;
      let srcAspect = width / Math.max(1, height);

      if (layer.media.kind === "video") {
        const vid = videoRefs.current[key];
        if (vid && vid.readyState >= 2) {
          source = vid;
          if (vid.videoWidth > 0 && vid.videoHeight > 0) {
            srcAspect = vid.videoWidth / vid.videoHeight;
          }
        }
      } else if (layer.media.kind === "image") {
        const img = imgRefs.current[key];
        if (img && img.complete && img.naturalWidth > 0) {
          source = img;
          srcAspect = img.naturalWidth / img.naturalHeight;
        }
      }

      if (!source) continue;

      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, tex);
      try {
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, source);
      } catch {
        continue;
      }

      const tr = layer.clip.transform;
      const isKenBurns =
        layer.clip.payload.kind === "media" && Boolean(layer.clip.payload.ken_burns);
      const localProgress = Math.max(
        0,
        Math.min(1, (nowSec - layer.clip.start_sec) / Math.max(0.1, layer.clip.duration_sec)),
      );
      const kbZoom = isKenBurns ? 1.0 + localProgress * 0.08 : 1.0;

      const reframe = tr.smart_reframe;
      const rx = reframe ? Math.max(0, Math.min(1, reframe.x)) : 0;
      const ry = reframe ? Math.max(0, Math.min(1, reframe.y)) : 0;
      const rw = reframe ? Math.max(0.05, Math.min(1 - rx, reframe.width)) : 1;
      const rh = reframe ? Math.max(0.05, Math.min(1 - ry, reframe.height)) : 1;

      gl.uniform2f(uniforms.u_translate, tr.x, tr.y);
      gl.uniform1f(uniforms.u_scale, Math.max(0.05, tr.scale) * kbZoom);
      gl.uniform1f(uniforms.u_rotation, (tr.rotation_deg * Math.PI) / 180);
      gl.uniform1f(uniforms.u_opacity, Math.max(0, Math.min(1, tr.opacity)));
      gl.uniform4f(uniforms.u_reframe, rx, ry, rw, rh);
      gl.uniform1f(uniforms.u_srcAspect, srcAspect);

      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
      drewAny = true;
    }

    const pixelHandler = onPixelFrameRef.current;
    if (drewAny && pixelHandler) {
      const now = performance.now();
      if (now - lastSampleTimeRef.current > 66) {
        lastSampleTimeRef.current = now;
        if (!sampleCanvasRef.current) {
          sampleCanvasRef.current = document.createElement("canvas");
          sampleCanvasRef.current.width = 160;
          sampleCanvasRef.current.height = 90;
        }
        const sCtx = sampleCanvasRef.current.getContext("2d", {
          willReadFrequently: true,
        });
        if (sCtx) {
          sCtx.drawImage(canvas, 0, 0, 160, 90);
          const imgData = sCtx.getImageData(0, 0, 160, 90);
          pixelHandler({ width: 160, height: 90, data: imgData.data });
        }
      }
    }
  }, [width, height]);

  const drawCurrentFrameRef = useRef(drawCurrentFrame);
  drawCurrentFrameRef.current = drawCurrentFrame;

  // Upload 3D LUT into native WebGL2 TEXTURE_3D ONLY when LUT or GL context changes
  useEffect(() => {
    const state = glStateRef.current;
    if (!state) return;
    const { gl, lutTex } = state;
    if (!brandLut || brandLut.size < 2) {
      state.hasLut = false;
      state.lutSize = 1;
      drawCurrentFrameRef.current();
      return;
    }
    const s = brandLut.size;
    const total = s * s * s;
    const rgba = new Uint8Array(total * 4);
    for (let i = 0; i < total; i++) {
      const srcIdx = i * 3;
      const dstIdx = i * 4;
      rgba[dstIdx] = Math.round(Math.max(0, Math.min(1, brandLut.data[srcIdx])) * 255);
      rgba[dstIdx + 1] = Math.round(Math.max(0, Math.min(1, brandLut.data[srcIdx + 1])) * 255);
      rgba[dstIdx + 2] = Math.round(Math.max(0, Math.min(1, brandLut.data[srcIdx + 2])) * 255);
      rgba[dstIdx + 3] = 255;
    }
    gl.bindTexture(gl.TEXTURE_3D, lutTex);
    gl.texImage3D(
      gl.TEXTURE_3D,
      0,
      gl.RGBA,
      s,
      s,
      s,
      0,
      gl.RGBA,
      gl.UNSIGNED_BYTE,
      rgba,
    );
    state.hasLut = true;
    state.lutSize = s;
    state.domainMin = brandLut.domain_min;
    state.domainMax = brandLut.domain_max;
    drawCurrentFrameRef.current();
  }, [brandLut, glRestoreSeq]);

  // Sync backing video elements cleanly (guard against seek starvation & pre-metadata seeks)
  useEffect(() => {
    for (const layer of playableLayers) {
      if (layer.media.kind !== "video") continue;
      const key = `${layer.trackId}:${layer.clip.id}`;
      const vid = videoRefs.current[key];
      if (!vid) continue;

      const rawTarget = Math.max(
        0,
        playheadSec - layer.clip.start_sec + layer.clip.source_in_sec,
      );
      const vidDur =
        Number.isFinite(vid.duration) && vid.duration > 0 ? vid.duration : null;
      const targetSec =
        vidDur !== null ? Math.min(rawTarget, Math.max(0, vidDur - 0.04)) : rawTarget;

      const threshold = isPlaying ? 0.22 : 0.015;
      if (vid.readyState < 1) {
        pendingVideoSeekRef.current[key] = targetSec;
      } else if (!vid.seeking && Math.abs(vid.currentTime - targetSec) > threshold) {
        delete pendingVideoSeekRef.current[key];
        try {
          vid.currentTime = targetSec;
        } catch {
          // Video metadata still loading
        }
      }

      if (isPlaying && vidDur !== null && rawTarget >= vidDur) {
        if (!vid.paused) vid.pause();
      } else if (isPlaying && vid.paused) {
        void vid.play().catch(() => {});
      } else if (!isPlaying && !vid.paused) {
        vid.pause();
      }
    }
  }, [playheadSec, isPlaying, playableLayers]);

  // Redraw when paused/scrubbing or grade/media changes (avoid duplicating rAF uploads while playing)
  useEffect(() => {
    if (!hasVisualMedia) {
      drawCurrentFrame();
      onPixelFrameRef.current?.(null);
      return;
    }
    if (!isPlaying) {
      drawCurrentFrame();
    }
  }, [hasVisualMedia, isPlaying, drawCurrentFrame, playheadSec, grade, playableLayers]);

  // Stable rAF redraw loop ONLY during active playback
  useEffect(() => {
    if (!isPlaying || !hasVisualMedia) return;
    let rafId = 0;
    const loop = () => {
      drawCurrentFrameRef.current();
      rafId = requestAnimationFrame(loop);
    };
    rafId = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(rafId);
  }, [isPlaying, hasVisualMedia]);

  // Listen for `solaris:gfx-ready` handshake from sandboxed GFX iframes
  useEffect(() => {
    const onMessage = (ev: MessageEvent) => {
      if (!ev.data || ev.data.type !== "solaris:gfx-ready") return;
      const srcWin = ev.source as Window | null;
      if (!srcWin) return;
      for (const clip of activeGfxRef.current) {
        const iframe = iframeRefs.current[clip.id];
        if (iframe?.contentWindow === srcWin) {
          srcWin.postMessage(
            {
              type: "solaris:seek",
              time: Math.max(0, playheadRef.current - clip.start_sec),
            },
            "*",
          );
          break;
        }
      }
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, []);

  // PostMessage seek to sandboxed GFX iframes
  useEffect(() => {
    for (const clip of activeGfxClips) {
      const iframe = iframeRefs.current[clip.id];
      if (!iframe?.contentWindow) continue;
      const localTime = Math.max(0, playheadSec - clip.start_sec);
      iframe.contentWindow.postMessage(
        { type: "solaris:seek", time: localTime },
        "*",
      );
    }
  }, [playheadSec, activeGfxClips]);

  // Stable GFX srcDoc generation (includes start_sec so moved clips re-sync cleanly)
  const gfxKeySignature = activeGfxClips
    .map((c) => {
      const gfx = c.payload.kind === "gfx" ? c.payload.gfx : null;
      return `${c.id}:${c.start_sec}:${c.duration_sec}:${c.label}:${gfx?.inline_html ?? ""}:${JSON.stringify(gfx?.props ?? {})}`;
    })
    .join("|");

  const gfxDocs = useMemo(() => {
    return activeGfxClips.map((clip) => {
      const html =
        clip.payload.kind === "gfx"
          ? clip.payload.gfx.inline_html ??
            `<div style="padding:12px 18px;border-radius:14px;background:rgba(8,4,4,0.85);border:1px solid var(--brand-accent);font-weight:600;">${escapeHtml(clip.label)}</div>`
          : "";
      const props = clip.payload.kind === "gfx" ? clip.payload.gfx.props : {};
      return {
        clipId: clip.id,
        startSec: clip.start_sec,
        transform: clip.transform,
        doc: buildSandboxedGfxDoc({
          htmlSource: html,
          props,
          brandTokens,
          fontCss: brandFontCss,
          durationSec: clip.duration_sec,
          initialTimeSec: 0,
        }),
      };
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [gfxKeySignature, brandTokens, brandFontCss]);

  const captionData: CaptionBlock | null =
    activeCaptionClip?.payload.kind === "caption"
      ? activeCaptionClip.payload.caption
      : null;

  const stageWellRef = useRef<HTMLDivElement | null>(null);
  const [stageWellSize, setStageWellSize] = useState<{ w: number; h: number }>({ w: 0, h: 0 });

  useEffect(() => {
    const el = stageWellRef.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const measure = () => {
      const rect = el.getBoundingClientRect();
      const w = Math.floor(rect.width);
      const h = Math.floor(rect.height);
      if (w <= 0 || h <= 0) return;
      setStageWellSize((prev) => (prev.w === w && prev.h === h ? prev : { w, h }));
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const aspectRatio = width / Math.max(1, height);
  const fittedDims = useMemo(() => {
    if (stageWellSize.w > 0 && stageWellSize.h > 0) {
      const wellRatio = stageWellSize.w / stageWellSize.h;
      const fitW =
        wellRatio > aspectRatio ? stageWellSize.h * aspectRatio : stageWellSize.w;
      const fitH = fitW / aspectRatio;
      return {
        w: Math.max(1, Math.floor(fitW)),
        h: Math.max(1, Math.floor(fitH)),
        measured: true,
      };
    }
    return { w: 0, h: 0, measured: false };
  }, [stageWellSize.w, stageWellSize.h, aspectRatio]);

  // Canonical virtual design coordinate space matching the canvas aspect ratio so GFX
  // overlays scale identically across narrow 9:16 monitors, widescreen panes, and exports.
  const gfxDesignSize = useMemo(() => {
    if (aspectRatio < 0.85) {
      return { w: 540, h: Math.round(540 / Math.max(0.25, aspectRatio)) };
    }
    if (aspectRatio > 1.2) {
      return { w: 960, h: Math.round(960 / aspectRatio) };
    }
    return { w: 720, h: Math.round(720 / Math.max(0.25, aspectRatio)) };
  }, [aspectRatio]);

  const gfxScale = fittedDims.measured ? fittedDims.w / gfxDesignSize.w : 1;

  const fittedStageStyle = useMemo(() => {
    if (fittedDims.measured) {
      return {
        width: `${fittedDims.w}px`,
        height: `${fittedDims.h}px`,
      };
    }
    return {
      aspectRatio: `${width} / ${height}`,
      maxHeight: "100%",
      maxWidth: "100%",
      width: aspectRatio >= 1 ? "100%" : "auto",
      height: aspectRatio >= 1 ? "auto" : "100%",
    };
  }, [fittedDims, aspectRatio, width, height]);

  return (
    <div
      ref={stageWellRef}
      className="relative flex h-full w-full items-center justify-center overflow-hidden"
    >
      <div
        className="relative flex items-center justify-center overflow-hidden rounded-xl border border-areia/15 bg-carbono/60 shadow-2xl"
        style={fittedStageStyle}
      >
      {/* Hidden backing media sources for V1 + V2 WebGL2 compositing */}
      {playableLayers.map((layer) => {
        const key = `${layer.trackId}:${layer.clip.id}`;
        if (layer.media.kind === "video" && layer.url) {
          return (
            <video
              key={key}
              ref={(el) => {
                videoRefs.current[key] = el;
                if (!el) delete pendingVideoSeekRef.current[key];
              }}
              src={layer.url}
              crossOrigin="anonymous"
              muted
              playsInline
              onLoadedMetadata={(e) => {
                const vid = e.currentTarget;
                const pending = pendingVideoSeekRef.current[key];
                if (pending !== undefined && !vid.seeking) {
                  delete pendingVideoSeekRef.current[key];
                  const dur =
                    Number.isFinite(vid.duration) && vid.duration > 0 ? vid.duration : null;
                  const clamped =
                    dur !== null ? Math.min(pending, Math.max(0, dur - 0.04)) : pending;
                  if (Math.abs(vid.currentTime - clamped) > 0.015) {
                    try {
                      vid.currentTime = clamped;
                    } catch {
                      // Ignore seek failure
                    }
                  }
                }
                drawCurrentFrame();
              }}
              onLoadedData={drawCurrentFrame}
              onSeeked={drawCurrentFrame}
              onCanPlay={drawCurrentFrame}
              className="hidden"
            />
          );
        }
        if (layer.media.kind === "image" && layer.url) {
          return (
            <img
              key={key}
              ref={(el) => {
                imgRefs.current[key] = el;
              }}
              src={layer.url}
              crossOrigin="anonymous"
              alt={layer.media.name}
              onLoad={drawCurrentFrame}
              className="hidden"
            />
          );
        }
        return null;
      })}

      {/* Persistently mounted WebGL2 Canvas */}
      <canvas
        ref={canvasRef}
        width={540}
        height={Math.round((540 * height) / Math.max(1, width))}
        className={`h-full w-full object-cover ${
          hasVisualMedia ? "opacity-100" : "pointer-events-none opacity-0"
        }`}
      />

      {!hasVisualMedia && activeGfxClips.length === 0 && !activeCaptionClip && (
        <div className="absolute inset-0 flex flex-col items-center justify-center gap-2 px-4 text-center">
          <Sparkles className="h-6 w-6 text-laranja-criacao" />
          <span className="text-xs font-medium text-areia/80">
            Solaris Program Stage
          </span>
          <span className="text-[10px] text-areia/45">
            Import media or direct Swarm to program visuals
          </span>
        </div>
      )}

      {/* Sandboxed HTML/GSAP GFX Layer (composited under T1 captions in canonical design coordinates) */}
      {gfxDocs.map(({ clipId, startSec, transform, doc }) => (
        <div
          key={clipId}
          className="pointer-events-none absolute top-0 left-0 origin-top-left"
          style={{
            width: `${gfxDesignSize.w}px`,
            height: `${gfxDesignSize.h}px`,
            transform: `scale(${gfxScale})`,
            opacity: transform.opacity,
          }}
        >
          <SandboxedGfxFrame
            client={client}
            ticketVersion={ticketVersion}
            doc={doc}
            title={`GFX ${clipId}`}
            iframeRef={(el) => {
              iframeRefs.current[clipId] = el;
            }}
            onLoad={(e) => {
              const win = e.currentTarget.contentWindow;
              if (win) {
                win.postMessage(
                  { type: "solaris:seek", time: Math.max(0, playheadRef.current - startSec) },
                  "*",
                );
              }
            }}
            style={{
              transform: `translate(${(transform.x * 100).toFixed(2)}%, ${(transform.y * 100).toFixed(2)}%) scale(${transform.scale}) rotate(${transform.rotation_deg}deg)`,
            }}
            className="pointer-events-none h-full w-full border-0 bg-transparent"
          />
        </div>
      ))}

      {/* Authentic 80% Title-Safe Word-Synced Captions (clear of bottom 22% social chrome) */}
      {activeCaptionClip && captionData && (
        <CaptionOverlay
          clip={activeCaptionClip}
          caption={captionData}
          playheadSec={playheadSec}
          stageWidth={fittedDims.measured ? fittedDims.w : 240}
          stageHeight={
            fittedDims.measured
              ? fittedDims.h
              : Math.round(240 / Math.max(0.25, aspectRatio))
          }
          isPortrait={height > width}
        />
      )}
      </div>
    </div>
  );
}

function resolveCaptionFontFamily(fontFamily?: string): string {
  const trimmed = fontFamily?.trim();
  if (
    !trimmed ||
    trimmed.toLowerCase() === "sora" ||
    trimmed.toLowerCase() === "sora variable"
  ) {
    return '"Sora Variable", "Sora", -apple-system, BlinkMacSystemFont, sans-serif';
  }
  const safe = trimmed.replace(/"/g, "");
  return `"${safe}", "Sora Variable", "Sora", -apple-system, BlinkMacSystemFont, sans-serif`;
}

function CaptionOverlay({
  clip,
  caption,
  playheadSec,
  stageWidth,
  stageHeight,
  isPortrait,
}: {
  clip: Clip;
  caption: CaptionBlock;
  playheadSec: number;
  stageWidth: number;
  stageHeight: number;
  isPortrait: boolean;
}) {
  // Align vertical clamp & font scaling with gfx_render.rs Chromium export
  const maxY = isPortrait ? 0.32 : 0.42;
  const maxTop = isPortrait ? 76 : 84;
  const clampedY = Math.max(-0.35, Math.min(maxY, clip.transform.y));
  const clampedX = Math.max(-0.3, Math.min(0.3, clip.transform.x));
  const topPct = Math.max(12, Math.min(maxTop, 50 + clampedY * 78));
  const leftPct = 50 + clampedX * 100;

  const words =
    caption.words.length > 0
      ? caption.words
      : [{ text: clip.label, start_sec: clip.start_sec, end_sec: clip.start_sec + clip.duration_sec }];

  const rawFontPx = isPortrait ? stageWidth * 0.052 : stageHeight * 0.054;
  const responsiveFontPx = Math.max(11, Math.min(24, Math.round(rawFontPx)));

  const presetBoxClass =
    caption.preset === "mono_caps"
      ? "rounded-lg bg-void/90 px-3 py-1 font-extrabold uppercase tracking-[0.12em] border border-areia/20 shadow-[0_8px_24px_rgba(0,0,0,0.65)]"
      : caption.preset === "karaoke"
        ? "rounded-full bg-carbono/90 px-3.5 py-1.5 font-extrabold tracking-tight border border-amarelo-ideias/35 shadow-[0_8px_26px_rgba(0,0,0,0.65)]"
        : caption.preset === "minimal"
          ? "rounded-lg bg-void/65 px-3 py-1 font-semibold tracking-normal shadow-[0_4px_16px_rgba(0,0,0,0.55)]"
          : "rounded-2xl bg-void/80 px-3.5 py-1.5 font-extrabold uppercase tracking-[0.02em] border border-areia/15 shadow-[0_10px_30px_rgba(0,0,0,0.7)]";

  return (
    <div
      className="pointer-events-none absolute z-10 w-[86%] text-center"
      style={{
        top: `${topPct}%`,
        left: `${leftPct}%`,
        transform: `translate(-50%, -50%) scale(${clip.transform.scale}) rotate(${clip.transform.rotation_deg}deg)`,
        opacity: clip.transform.opacity,
        fontFamily: resolveCaptionFontFamily(caption.font_family),
        fontSize: `${responsiveFontPx}px`,
        lineHeight: 1.15,
        textWrap: "balance",
      }}
    >
      <div className={`inline-flex max-w-full flex-wrap items-center justify-center gap-x-[0.28em] gap-y-0.5 ${presetBoxClass}`}>
        {words.map((w, idx) => {
          const isActive =
            playheadSec >= w.start_sec && playheadSec <= Math.max(w.end_sec, w.start_sec + 0.05);
          const isSpoken = playheadSec > w.end_sec;
          const activeColor = caption.highlight_color || "#F7B733";
          return (
            <span
              key={`${w.text}-${idx}`}
              style={
                isActive
                  ? {
                      color: caption.preset === "karaoke" ? "#080404" : activeColor,
                      backgroundColor:
                        caption.preset === "karaoke" ? activeColor : "transparent",
                      textShadow:
                        caption.preset === "neon_glow"
                          ? `0 0 16px ${activeColor}, 0 0 4px ${activeColor}`
                          : undefined,
                    }
                  : {
                      color:
                        caption.preset === "karaoke" && isSpoken
                          ? activeColor
                          : "#F7F5EF",
                      opacity: caption.preset === "minimal" && !isActive ? 0.84 : 0.94,
                    }
              }
              className={`rounded px-0.5 transition-transform duration-100 motion-reduce:transition-none ${
                isActive && caption.preset !== "minimal" ? "scale-105" : ""
              }`}
            >
              {w.text}
            </span>
          );
        })}
      </div>
    </div>
  );
}
