import type { Clip, MediaAsset, Track } from "@solaris/engine-client";

export interface AudioBusSource {
  clip: Clip;
  bus: "voice" | "music";
  streamUrl: string;
}

export interface LiveAudioLevels {
  voicePct: number;
  musicPct: number;
  momentaryLufs: number | null;
}

interface ClipAudioSlot {
  clipId: string;
  bus: "voice" | "music";
  url: string;
  el: HTMLAudioElement;
  sourceNode: MediaElementAudioSourceNode;
  gainNode: GainNode;
  pendingSeekSec: number | null;
  onLoadedMetadata: () => void;
}

function dbToLinear(db: number): number {
  if (!Number.isFinite(db) || db <= -80) return 0;
  return Math.min(4.0, Math.pow(10, db / 20));
}

function computeClipEnvelopeGain(clip: Clip, playheadSec: number): number {
  const localT = playheadSec - clip.start_sec;
  const dur = Math.max(0.05, clip.duration_sec);
  let env = 1.0;
  const fadeIn = Math.max(0, clip.audio.fade_in_sec);
  const fadeOut = Math.max(0, clip.audio.fade_out_sec);
  if (fadeIn > 0.001 && localT < fadeIn) {
    env *= Math.max(0, Math.min(1, localT / fadeIn));
  }
  const remain = dur - localT;
  if (fadeOut > 0.001 && remain < fadeOut) {
    env *= Math.max(0, Math.min(1, remain / fadeOut));
  }
  return env;
}

/**
 * Playhead-synchronized Web Audio API playback & live metering engine.
 * Routes active A1 (Voice), V1 (A-Roll audio when unmuted & no explicit voice override),
 * and A2 (Music with live voice-driven sidechain ducking) through AnalyserNodes.
 */
export class SolarisAudioEngine {
  private ctx: AudioContext | null = null;
  private voiceBus: GainNode | null = null;
  private musicBus: GainNode | null = null;
  private voiceAnalyser: AnalyserNode | null = null;
  private musicAnalyser: AnalyserNode | null = null;
  private voiceBuf: Float32Array<ArrayBuffer> | null = null;
  private musicBuf: Float32Array<ArrayBuffer> | null = null;
  private readonly slots = new Map<string, ClipAudioSlot>();
  private disposed = false;

  private ensureContext(): boolean {
    if (this.disposed || typeof window === "undefined") return false;
    if (this.ctx) {
      if (this.ctx.state === "suspended") {
        void this.ctx.resume().catch(() => {});
      }
      return true;
    }
    const Ctor =
      window.AudioContext ||
      (window as unknown as { webkitAudioContext?: typeof AudioContext })
        .webkitAudioContext;
    if (!Ctor) return false;

    try {
      const ctx = new Ctor();
      const voiceBus = ctx.createGain();
      const musicBus = ctx.createGain();
      const voiceAnalyser = ctx.createAnalyser();
      const musicAnalyser = ctx.createAnalyser();
      voiceAnalyser.fftSize = 512;
      musicAnalyser.fftSize = 512;

      voiceBus.connect(voiceAnalyser);
      voiceAnalyser.connect(ctx.destination);

      musicBus.connect(musicAnalyser);
      musicAnalyser.connect(ctx.destination);

      this.ctx = ctx;
      this.voiceBus = voiceBus;
      this.musicBus = musicBus;
      this.voiceAnalyser = voiceAnalyser;
      this.musicAnalyser = musicAnalyser;
      this.voiceBuf = new Float32Array(voiceAnalyser.fftSize);
      this.musicBuf = new Float32Array(musicAnalyser.fftSize);
      return true;
    } catch {
      return false;
    }
  }

  public unlock(): void {
    if (!this.ensureContext() || !this.ctx) return;
    if (this.ctx.state === "suspended") {
      void this.ctx.resume().catch(() => {});
    }
  }

  private getOrCreateSlot(item: AudioBusSource): ClipAudioSlot | null {
    if (!this.ctx || !this.voiceBus || !this.musicBus) return null;
    const existing = this.slots.get(item.clip.id);
    if (existing && existing.url === item.streamUrl && existing.bus === item.bus) {
      return existing;
    }
    if (existing) {
      this.destroySlot(item.clip.id);
    }

    const el = document.createElement("audio");
    el.crossOrigin = "anonymous";
    el.preload = "auto";
    el.src = item.streamUrl;

    try {
      const sourceNode = this.ctx.createMediaElementSource(el);
      const gainNode = this.ctx.createGain();
      gainNode.gain.value = 0;
      sourceNode.connect(gainNode);
      gainNode.connect(item.bus === "voice" ? this.voiceBus : this.musicBus);
      const slot: ClipAudioSlot = {
        clipId: item.clip.id,
        bus: item.bus,
        url: item.streamUrl,
        el,
        sourceNode,
        gainNode,
        pendingSeekSec: null,
        onLoadedMetadata: () => {
          if (slot.pendingSeekSec === null || el.seeking) return;
          const raw = slot.pendingSeekSec;
          slot.pendingSeekSec = null;
          const dur =
            Number.isFinite(el.duration) && el.duration > 0 ? el.duration : null;
          const clamped = dur !== null ? Math.min(raw, Math.max(0, dur - 0.05)) : raw;
          if (Math.abs(el.currentTime - clamped) > 0.05) {
            try {
              el.currentTime = clamped;
            } catch {
              // Ignore seek failure
            }
          }
        },
      };
      el.addEventListener("loadedmetadata", slot.onLoadedMetadata);
      this.slots.set(item.clip.id, slot);
      return slot;
    } catch {
      return null;
    }
  }

  private destroySlot(clipId: string): void {
    const slot = this.slots.get(clipId);
    if (!slot) return;
    try {
      slot.el.removeEventListener("loadedmetadata", slot.onLoadedMetadata);
      slot.el.pause();
      slot.el.removeAttribute("src");
      slot.el.load();
      slot.sourceNode.disconnect();
      slot.gainNode.disconnect();
    } catch {
      // Ignore cleanup errors
    }
    this.slots.delete(clipId);
  }

  sync(params: {
    tracks: Track[];
    mediaPool: MediaAsset[];
    playheadSec: number;
    isPlaying: boolean;
    getStreamUrl: (mediaId: string) => string;
  }): void {
    const { tracks, mediaPool, playheadSec, isPlaying, getStreamUrl } = params;
    const audioTracks = tracks.filter(
      (t) => t.id === "a1" || t.id === "a2" || t.id === "v1",
    );
    const anySolo = audioTracks.some((t) => t.solo);
    const trackAudible = (t: Track | undefined) =>
      Boolean(t && !t.muted && (!anySolo || t.solo));

    const a1 = tracks.find((t) => t.id === "a1");
    const v1 = tracks.find((t) => t.id === "v1");
    const a2 = tracks.find((t) => t.id === "a2");

    const activeSources: AudioBusSource[] = [];
    const activeAt = (t: Track | undefined) =>
      trackAudible(t)
        ? (t?.clips.filter(
            (c) =>
              playheadSec >= c.start_sec &&
              playheadSec < c.start_sec + c.duration_sec &&
              c.audio.gain_db > -80,
          ) ?? [])
        : [];

    const activeA1 = activeAt(a1);
    const activeV1 = activeAt(v1);
    const activeA2 = activeAt(a2);

    const resolvePlayableMedia = (clip: Clip): MediaAsset | null => {
      if (clip.payload.kind !== "media") return null;
      const mediaId = clip.payload.media_id;
      const asset = mediaPool.find((m) => m.id === mediaId);
      if (!asset || !asset.path) return null;
      if (asset.kind === "image" || asset.kind === "document") return null;
      if (asset.kind === "video" && asset.intelligence.has_audio === false) {
        return null;
      }
      return asset;
    };

    let hasActiveA1Voice = false;
    let hasActiveVoice = false;
    for (const clip of activeA1) {
      const asset = resolvePlayableMedia(clip);
      if (asset) {
        hasActiveA1Voice = true;
        hasActiveVoice = true;
        activeSources.push({
          clip,
          bus: "voice",
          streamUrl: getStreamUrl(asset.id),
        });
      }
    }

    // Deduplicate overlapping V1 audio when an explicit A1 voice clip is active (unless V1 is soloed)
    if (!hasActiveA1Voice || Boolean(v1?.solo)) {
      for (const clip of activeV1) {
        const asset = resolvePlayableMedia(clip);
        if (asset) {
          hasActiveVoice = true;
          activeSources.push({
            clip,
            bus: "voice",
            streamUrl: getStreamUrl(asset.id),
          });
        }
      }
    }

    for (const clip of activeA2) {
      const asset = resolvePlayableMedia(clip);
      if (asset) {
        activeSources.push({
          clip,
          bus: "music",
          streamUrl: getStreamUrl(asset.id),
        });
      }
    }

    // Pause and prune inactive slots cleanly (cancel scheduled AudioParam curves first)
    const activeIds = new Set(activeSources.map((s) => s.clip.id));
    const currentAudioTime = this.ctx?.currentTime ?? 0;
    for (const [clipId, slot] of this.slots.entries()) {
      if (!activeIds.has(clipId)) {
        if (this.ctx) {
          slot.gainNode.gain.cancelScheduledValues(currentAudioTime);
          slot.gainNode.gain.setTargetAtTime(0, currentAudioTime, 0.015);
        }
        if (!slot.el.paused) slot.el.pause();
        // Garbage-collect slots removed from timeline
        const stillInTimeline = audioTracks.some((t) =>
          t.clips.some((c) => c.id === clipId),
        );
        if (!stillInTimeline) {
          this.destroySlot(clipId);
        }
      }
    }

    if (!isPlaying) {
      for (const slot of this.slots.values()) {
        if (this.ctx) {
          slot.gainNode.gain.cancelScheduledValues(currentAudioTime);
          slot.gainNode.gain.setTargetAtTime(0, currentAudioTime, 0.015);
        }
        if (!slot.el.paused) slot.el.pause();
      }
      return;
    }

    if (activeSources.length === 0) return;
    if (!this.ensureContext() || !this.ctx) return;

    const now = this.ctx.currentTime;
    for (const item of activeSources) {
      const slot = this.getOrCreateSlot(item);
      if (!slot) continue;

      const rawTarget = Math.max(
        0,
        item.clip.source_in_sec + (playheadSec - item.clip.start_sec),
      );
      const dur = Number.isFinite(slot.el.duration) && slot.el.duration > 0
        ? slot.el.duration
        : null;
      const targetSec = dur !== null ? Math.min(rawTarget, Math.max(0, dur - 0.05)) : rawTarget;

      if (dur !== null && rawTarget >= dur) {
        slot.gainNode.gain.cancelScheduledValues(now);
        slot.gainNode.gain.setTargetAtTime(0, now, 0.015);
        if (!slot.el.paused) slot.el.pause();
        continue;
      }

      if (slot.el.readyState < 1) {
        slot.pendingSeekSec = targetSec;
      } else if (
        !slot.el.seeking &&
        Math.abs(slot.el.currentTime - targetSec) > 0.22
      ) {
        slot.pendingSeekSec = null;
        try {
          slot.el.currentTime = targetSec;
        } catch {
          // Element may still be loading metadata
        }
      }

      const env = computeClipEnvelopeGain(item.clip, playheadSec);
      let gainDb = item.clip.audio.gain_db;
      if (item.bus === "music" && hasActiveVoice && item.clip.audio.ducking_enabled) {
        const duckTarget = Number.isFinite(item.clip.audio.duck_target_db)
          ? item.clip.audio.duck_target_db
          : -18;
        gainDb = Math.min(gainDb, duckTarget);
      }
      const linearGain = dbToLinear(gainDb) * env;
      slot.gainNode.gain.cancelScheduledValues(now);
      slot.gainNode.gain.setTargetAtTime(linearGain, now, 0.03);

      if (slot.el.paused) {
        void slot.el.play().catch(() => {});
      }
    }
  }

  getLiveLevels(): LiveAudioLevels {
    if (!this.voiceAnalyser || !this.musicAnalyser || !this.voiceBuf || !this.musicBuf) {
      return { voicePct: 0, musicPct: 0, momentaryLufs: null };
    }
    const sampleRmsDb = (
      analyser: AnalyserNode,
      buf: Float32Array<ArrayBuffer>,
    ): number => {
      analyser.getFloatTimeDomainData(buf);
      let sum = 0;
      for (let i = 0; i < buf.length; i++) {
        sum += buf[i] * buf[i];
      }
      const rms = Math.sqrt(sum / Math.max(1, buf.length));
      if (rms < 1e-5) return -72;
      return 20 * Math.log10(rms);
    };

    const voiceDb = sampleRmsDb(this.voiceAnalyser, this.voiceBuf);
    const musicDb = sampleRmsDb(this.musicAnalyser, this.musicBuf);
    const dbToPct = (db: number) =>
      Math.max(0, Math.min(100, ((db + 54) / 54) * 100));

    const combinedLin =
      Math.pow(10, voiceDb / 10) + Math.pow(10, musicDb / 10);
    const combinedDb = combinedLin > 1e-7 ? 10 * Math.log10(combinedLin) : null;

    return {
      voicePct: voiceDb <= -68 ? 0 : dbToPct(voiceDb),
      musicPct: musicDb <= -68 ? 0 : dbToPct(musicDb),
      momentaryLufs:
        combinedDb !== null && combinedDb > -65 ? combinedDb - 0.691 : null,
    };
  }

  dispose(): void {
    this.disposed = true;
    for (const clipId of Array.from(this.slots.keys())) {
      this.destroySlot(clipId);
    }
    if (this.ctx) {
      void this.ctx.close().catch(() => {});
      this.ctx = null;
    }
    this.voiceBus = null;
    this.musicBus = null;
    this.voiceAnalyser = null;
    this.musicAnalyser = null;
  }
}
