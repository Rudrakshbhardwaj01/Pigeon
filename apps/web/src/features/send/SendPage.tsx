import { useCallback, useEffect, useRef, useState } from "react";
import { ArrowRight, Download, File as FileIcon, Gauge, Lock, LockOpen, Settings, X } from "lucide-react";

import { Dropzone } from "../../components/Dropzone";
import { ErrorDisplay } from "../../components/ErrorDisplay";
import { ProgressBar, Stat } from "../../components/Stats";
import { downloadBlob } from "../../lib/download";
import { formatBytes, formatDuration } from "../../lib/format";
import { WorkerClient } from "../../lib/worker-client";
import type { EncodeRequest, EncodeResponse } from "../../workers/encode.worker";
import type { EncodeStats, TransferEstimate } from "../../wasm/bridge";

/**
 * Transfer presets: named bundles of real engine parameters. "Custom" is
 * selected automatically whenever any control is changed by hand.
 */
const PRESETS = {
  balanced: {
    label: "Balanced",
    hint: "20% redundancy · 4 fps · auto compression",
    overhead: 20,
    delayMs: 250,
    compression: "auto" as const,
  },
  reliability: {
    label: "Maximum reliability",
    hint: "50% redundancy · easy 2.5 fps scanning",
    overhead: 50,
    delayMs: 400,
    compression: "auto" as const,
  },
  fast: {
    label: "Faster transmission",
    hint: "10% redundancy · 8 fps · compression always on",
    overhead: 10,
    delayMs: 120,
    compression: "on" as const,
  },
  camera: {
    label: "Camera-friendly",
    hint: "30% redundancy · easy scanning for live capture",
    overhead: 30,
    delayMs: 400,
    compression: "auto" as const,
  },
} as const;

type PresetId = keyof typeof PRESETS | "custom";

interface EncodeProgressState {
  phase: "preparing" | "fountain" | "qr-gif" | "ready";
  framesDone?: number;
  totalFrames?: number;
}

interface GifResult {
  gif: ArrayBuffer;
  stats: EncodeStats;
  url: string;
  durationMs: number;
}

/** Honest, length-based passphrase hint. Never claims true entropy. */
function passphraseStrength(pw: string): { label: string; tone: "info" | "warning" } {
  if (pw.length === 0) {
    return { label: "Passphrase required for an encrypted transfer.", tone: "info" };
  }
  if (pw.length < 12) {
    return { label: "Short passphrases are guessable — 12+ characters recommended.", tone: "warning" };
  }
  if (pw.length < 20) {
    return { label: "Reasonable. A long unique phrase is stronger.", tone: "info" };
  }
  return { label: "Strong length. Share it over a channel the GIF does not travel on.", tone: "info" };
}

export function SendPage() {
  const [file, setFile] = useState<File | null>(null);
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<EncodeProgressState | null>(null);
  const [result, setResult] = useState<GifResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [cancelled, setCancelled] = useState(false);
  const [overhead, setOverhead] = useState(20);
  const [delayMs, setDelayMs] = useState(250);
  const [compression, setCompression] = useState<"auto" | "on" | "off">("auto");
  const [encrypt, setEncrypt] = useState(false);
  const [passphrase, setPassphrase] = useState("");
  const [passphraseConfirm, setPassphraseConfirm] = useState("");
  const [showPassphrase, setShowPassphrase] = useState(false);
  const [passphraseError, setPassphraseError] = useState<string | null>(null);
  const [presetId, setPresetId] = useState<PresetId>("balanced");
  const [estimate, setEstimate] = useState<TransferEstimate | null>(null);

  const clientRef = useRef<WorkerClient<EncodeRequest, EncodeResponse> | null>(null);
  const resultUrlRef = useRef<string | null>(null);
  useEffect(() => {
    const client = new WorkerClient<EncodeRequest, EncodeResponse>(
      () => new Worker(new URL("../../workers/encode.worker.ts", import.meta.url), { type: "module" })
    );
    clientRef.current = client;
    return () => {
      client.terminate();
      clientRef.current = null;
      if (resultUrlRef.current) {
        URL.revokeObjectURL(resultUrlRef.current);
        resultUrlRef.current = null;
      }
    };
  }, []);

  const pickFile = useCallback((picked: File) => {
    setResult(null);
    setError(null);
    setProgress(null);
    setCancelled(false);
    setFile(picked);
  }, []);

  const cancel = useCallback(() => {
    setCancelled(true);
    clientRef.current?.send({ kind: "cancel" });
  }, []);

  const toggleEncrypt = useCallback(() => {
    setEncrypt((v) => !v);
    setPassphrase("");
    setPassphraseConfirm("");
    setPassphraseError(null);
  }, []);

  /** Applies a preset's real parameters and marks it active. */
  const applyPreset = useCallback((id: keyof typeof PRESETS) => {
    const p = PRESETS[id];
    setOverhead(p.overhead);
    setDelayMs(p.delayMs);
    setCompression(p.compression);
    setPresetId(id);
  }, []);

  /** Manual control changes select "custom" (no preset is lying). */
  const markCustom = useCallback(() => setPresetId("custom"), []);

  // Pre-encode estimate: cheap (QR-capacity probing only), debounced, and
  // refreshed whenever any parameter that changes geometry moves.
  useEffect(() => {
    if (!file) {
      setEstimate(null);
      return;
    }
    const client = clientRef.current;
    if (!client) {
      return;
    }
    const timer = setTimeout(() => {
      void client
        .call<{ estimate: TransferEstimate }>({
          kind: "estimate",
          size: file.size,
          params: {
            filename: file.name,
            compression,
            overheadPercent: overhead,
            frameDelayMs: delayMs,
            ...(encrypt && passphrase ? { passphrase } : {}),
          },
        } as Omit<EncodeRequest, "id">)
        .then((res) => setEstimate(res.estimate))
        .catch(() => setEstimate(null));
    }, 250);
    return () => clearTimeout(timer);
  }, [file, overhead, delayMs, compression, encrypt, passphrase]);

  const encode = useCallback(async () => {
    if (!file || !clientRef.current) {
      return;
    }
    if (encrypt && passphrase.length === 0) {
      setPassphraseError("Enter a passphrase, or turn encryption off.");
      return;
    }
    if (encrypt && passphrase !== passphraseConfirm) {
      setPassphraseError("The two passphrases do not match.");
      return;
    }
    setPassphraseError(null);
    setBusy(true);
    setError(null);
    setCancelled(false);
    setResult(null);
    setProgress({ phase: "preparing" });
    const started = performance.now();
    try {
      const data = await file.arrayBuffer();
      const res = await clientRef.current.call<{ gif: ArrayBuffer; stats: EncodeStats }>(
        {
          kind: "encode-gif",
          data,
          params: {
            filename: file.name,
            mimeType: file.type || null,
            compression,
            overheadPercent: overhead,
            frameDelayMs: delayMs,
            ...(encrypt && passphrase ? { passphrase } : {}),
          },
        } as Omit<EncodeRequest, "id">,
        (message) => {
          const m = message as EncodeProgressState & { kind: string };
          if (m.kind === "progress") {
            setProgress({ phase: m.phase, framesDone: m.framesDone, totalFrames: m.totalFrames });
          }
        }
      );
      const blob = new Blob([res.gif], { type: "image/gif" });
      const url = URL.createObjectURL(blob);
      resultUrlRef.current = url;
      setResult({
        gif: res.gif,
        stats: res.stats,
        url,
        durationMs: performance.now() - started,
      });
      setProgress({ phase: "ready" });
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      if (message === "cancelled") {
        setCancelled(true);
        setProgress(null);
      } else {
        setError(message);
        setProgress(null);
      }
    } finally {
      setBusy(false);
    }
  }, [file, overhead, delayMs, compression, encrypt, passphrase, passphraseConfirm]);

  const reset = useCallback(() => {
    if (resultUrlRef.current) {
      URL.revokeObjectURL(resultUrlRef.current);
      resultUrlRef.current = null;
    }
    setResult(null);
    setProgress(null);
    setError(null);
    setCancelled(false);
    // The passphrase is intentionally kept so a re-encode of a new file with
    // the same settings does not require retyping it; clear it on explicit
    // encryption toggle instead.
  }, []);

  const progressLabel =
    progress?.phase === "preparing"
      ? "Reading and preparing file"
      : progress?.phase === "fountain"
        ? "Fountain-encoding packets"
        : progress?.phase === "qr-gif"
          ? "Rendering QR frames into GIF"
          : null;

  const progressValue =
    progress?.phase === "qr-gif" && progress.totalFrames
      ? (progress.framesDone ?? 0) / progress.totalFrames
      : progress?.phase === "ready"
        ? 1
        : progress
          ? null
          : null;

  return (
    <section className="section" id="send" aria-labelledby="send-heading">
      {!result ? (
        <div className="card section-card">
          <header className="section-header">
            <h2 id="send-heading" className="section-title">
              Send a File
            </h2>
            <p className="page-sub">
              Turn any file into an animated QR code (GIF) for offline transfer.
            </p>
          </header>

          <Dropzone
            label="Drag &amp; drop a file here"
            hint="or click to browse — processed entirely on this device"
            onFile={pickFile}
            disabled={busy}
          />

          {file ? (
            <div className="file-card" aria-label="Selected file">
              <FileIcon size={20} strokeWidth={1.5} className="file-icon" />
              <div className="file-meta">
                <strong className="file-name">{file.name}</strong>
                <span className="file-sub">
                  {formatBytes(file.size)}
                  {file.type ? ` · ${file.type}` : ""}
                </span>
              </div>
              <button
                className="btn ghost small"
                onClick={() => {
                  setFile(null);
                  setProgress(null);
                  setError(null);
                }}
                disabled={busy}
              >
                <X size={14} /> Remove
              </button>
            </div>
          ) : null}

          {file && estimate ? (
            <div className="estimate-strip" role="status">
              <Gauge size={15} strokeWidth={1.6} />
              <span>
                <strong>Estimate:</strong> up to ≈{estimate.framesIncompressible} frames
                {estimate.framesCompressibleOptimistic < estimate.framesIncompressible
                  ? ` (≈${estimate.framesCompressibleOptimistic} if the content compresses well)`
                  : ""}{" "}
                · ≈{formatBytes(estimate.gifBytesEstimate)} GIF · ≈
                {formatDuration(estimate.loopSeconds * 1000)} per loop · QR v{estimate.qrVersion}
                -{estimate.qrEcl.toUpperCase()}
                {estimate.encrypted ? " · encrypted geometry" : ""}
              </span>
            </div>
          ) : null}

          <div className="settings-card">
            <div className="settings-title">
              <Settings size={15} /> Transfer Settings
            </div>
            <div className="preset-row" role="radiogroup" aria-label="Transfer preset">
              {(Object.keys(PRESETS) as Array<keyof typeof PRESETS>).map((id) => (
                <button
                  key={id}
                  type="button"
                  role="radio"
                  aria-checked={presetId === id}
                  className={`preset${presetId === id ? " active" : ""}`}
                  onClick={() => applyPreset(id)}
                  disabled={busy}
                  title={PRESETS[id].hint}
                >
                  {PRESETS[id].label}
                </button>
              ))}
              <span
                role="radio"
                aria-checked={presetId === "custom"}
                className={`preset custom${presetId === "custom" ? " active" : ""}`}
              >
                Custom
              </span>
            </div>
            <div className="settings-grid">
              <label className="field">
                <span>Redundancy (overhead)</span>
                <select
                  value={overhead}
                  onChange={(e) => {
                    setOverhead(Number(e.target.value));
                    markCustom();
                  }}
                  disabled={busy}
                >
                  <option value={10}>10% — tight GIF</option>
                  <option value={20}>20% — recommended</option>
                  <option value={30}>30% — resilient</option>
                  <option value={50}>50% — very resilient</option>
                  <option value={100}>100% — maximum tolerance</option>
                </select>
              </label>
              <label className="field">
                <span>Frame rate</span>
                <select
                  value={delayMs}
                  onChange={(e) => {
                    setDelayMs(Number(e.target.value));
                    markCustom();
                  }}
                  disabled={busy}
                >
                  <option value={120}>120 ms — 8 fps fast capture</option>
                  <option value={180}>180 ms — ~5.5 fps</option>
                  <option value={250}>250 ms — 4 fps default</option>
                  <option value={400}>400 ms — easy scanning</option>
                </select>
              </label>
              <label className="field">
                <span>Compression</span>
                <select
                  value={compression}
                  onChange={(e) => {
                    setCompression(e.target.value as "auto" | "on" | "off");
                    markCustom();
                  }}
                  disabled={busy}
                >
                  <option value="auto">Auto (skip media)</option>
                  <option value="on">Always</option>
                  <option value="off">Off</option>
                </select>
              </label>
            </div>

            <div className="encrypt-block">
              <button
                type="button"
                className={`encrypt-toggle${encrypt ? " on" : ""}`}
                onClick={toggleEncrypt}
                disabled={busy}
                aria-pressed={encrypt}
              >
                {encrypt ? <Lock size={15} /> : <LockOpen size={15} />}
                <span>
                  {encrypt ? "Encrypted transfer" : "Encryption off — the GIF holds the file in plaintext"}
                </span>
                <span className={`encrypt-state${encrypt ? " on" : ""}`}>{encrypt ? "On" : "Off"}</span>
              </button>
              {encrypt ? (
                <div className="encrypt-fields">
                  <div className="field">
                    <span>Passphrase</span>
                    <div className="passphrase-row">
                      <input
                        type={showPassphrase ? "text" : "password"}
                        value={passphrase}
                        onChange={(e) => {
                          setPassphrase(e.target.value);
                          setPassphraseError(null);
                        }}
                        autoComplete="new-password"
                        disabled={busy}
                        aria-label="Passphrase"
                        placeholder="Shared separately from the GIF"
                      />
                      <button
                        type="button"
                        className="btn ghost small"
                        onClick={() => setShowPassphrase((v) => !v)}
                        aria-label={showPassphrase ? "Hide passphrase" : "Show passphrase"}
                        disabled={busy}
                      >
                        {showPassphrase ? "Hide" : "Show"}
                      </button>
                    </div>
                  </div>
                  <div className="field">
                    <span>Confirm passphrase</span>
                    <input
                      type={showPassphrase ? "text" : "password"}
                      value={passphraseConfirm}
                      onChange={(e) => {
                        setPassphraseConfirm(e.target.value);
                        setPassphraseError(null);
                      }}
                      autoComplete="new-password"
                      disabled={busy}
                      aria-label="Confirm passphrase"
                      placeholder="Repeat it exactly"
                    />
                  </div>
                  {(() => {
                    const s = passphraseStrength(passphrase);
                    return (
                      <p className={`hint passphrase-hint${s.tone === "warning" ? " warn" : ""}`}>
                        {s.label}
                      </p>
                    );
                  })()}
                  {passphraseError ? (
                    <p className="hint passphrase-hint warn" role="alert">
                      {passphraseError}
                    </p>
                  ) : null}
                  <p className="hint passphrase-hint">
                    Encrypted transfers use Argon2id + XChaCha20-Poly1305: the file,
                    filename and metadata are unreadable without the passphrase. The
                    passphrase travels separately — never inside the GIF.
                  </p>
                </div>
              ) : null}
            </div>
          </div>

          {error ? <ErrorDisplay title="Encoding failed" detail={error} /> : null}
          {cancelled ? (
            <div className="notice notice-warning" role="status">
              <div>
                <div className="notice-title">Encoding cancelled</div>
                <div className="notice-detail">
                  No GIF was produced. You can adjust the file or settings and encode again.
                </div>
              </div>
            </div>
          ) : null}

          {busy && progress ? (
            <div className="progress-block">
              <ProgressBar
                value={progressValue}
                label={progressLabel ?? "Working"}
                detail={
                  progress.totalFrames
                    ? `Frame ${progress.framesDone ?? 0} of ${progress.totalFrames}`
                    : undefined
                }
              />
              <div className="actions">
                <button className="btn ghost" onClick={cancel}>
                  Cancel
                </button>
              </div>
            </div>
          ) : null}

          <div className="actions">
            <button className="btn primary" onClick={() => void encode()} disabled={!file || busy}>
              <ArrowRight size={16} /> Encode to GIF
            </button>
          </div>
        </div>
      ) : (
        <EncodeResultView result={result} onReset={reset} />
      )}
    </section>
  );
}

function EncodeResultView({ result, onReset }: { result: GifResult; onReset: () => void }) {
  const s = result.stats;
  const gifBytes = result.gif.byteLength;

  return (
    <div className="card section-card">
      <header className="section-header">
        <h2 className="section-title">Transfer ready</h2>
        <p className="page-sub">
          Send this GIF as a <strong>document / file attachment</strong> (WhatsApp, Telegram,
          e-mail, Drive, USB…). Messaging apps that re-encode media as video may damage frames —
          the built-in redundancy tolerates some loss, but the original file is always safest.
        </p>
      </header>

      <div className="gif-card">
        <img className="gif-preview" src={result.url} alt="Animated QR transfer (first frame)" />
        <div className="gif-actions">
          <button
            className="btn primary"
            onClick={() => downloadBlob(result.gif, `pigeon-${s.filename.replace(/[^\w.-]+/g, "_")}.gif`, "image/gif")}
          >
            <Download size={16} /> Download GIF ({formatBytes(gifBytes)})
          </button>
          <button className="btn ghost" onClick={onReset}>
            Encode another file
          </button>
        </div>
        <p className="hint">The downloaded GIF retains its complete animation and loops forever.</p>
      </div>

      <div className="stats-card" aria-label="Transfer statistics">
        <h3 className="stats-title">Transfer details</h3>
        <div className="stats-grid">
          <Stat label="File" value={s.filename} />
          <Stat label="Original size" value={formatBytes(s.originalSize)} />
          <Stat label="Compressed size" value={formatBytes(s.encodedSize)} />
          <Stat
            label="Compression"
            value={s.compressionUsed ? "zlib (applied)" : "none"}
          />
          <Stat
            label="Encryption"
            value={s.encrypted ? "Argon2id + XChaCha20-Poly1305" : "none (plaintext GIF)"}
          />
          <Stat label="QR frames" value={String(s.totalFrames)} mono />
          <Stat label="Required packets" value={String(s.neededPackets)} mono />
          <Stat label="Redundant packets" value={String(s.repairSymbols)} mono />
          <Stat label="QR version / ECL" value={`${s.qrVersion} · ${s.qrEcl.toUpperCase()}`} mono />
          <Stat label="Payload / frame" value={formatBytes(s.packetPayloadBytes)} mono />
          <Stat label="Loop duration" value={formatDuration(estimateLoop(s))} mono />
          <Stat label="Encode time" value={formatDuration(result.durationMs)} mono />
          <Stat label="SHA-256" value={`${s.originalSha256.slice(0, 16)}…`} mono />
        </div>
        <p className="hint mono selectable">{s.originalSha256}</p>
      </div>
    </div>
  );
}

function estimateLoop(s: EncodeStats): number {
  return s.totalFrames * s.frameDelayMs;
}
