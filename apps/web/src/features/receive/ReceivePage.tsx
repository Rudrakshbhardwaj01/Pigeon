import { useCallback, useEffect, useRef, useState } from "react";
import { Camera, CameraOff, Download, FileImage, KeyRound, RefreshCw } from "lucide-react";

import { Dropzone } from "../../components/Dropzone";
import { ErrorDisplay } from "../../components/ErrorDisplay";
import { ProgressBar, Stat } from "../../components/Stats";
import { downloadBlob } from "../../lib/download";
import { formatBytes, humanPhase } from "../../lib/format";
import { WorkerClient } from "../../lib/worker-client";
import type { DecodeRequest, DecodeResponse } from "../../workers/decode.worker";
import type { ReconstructedFile, TransferStatus } from "../../wasm/bridge";

interface DecodeProgressState {
  phase: "parsing" | "collecting" | "verifying" | "done";
  framesSeen: number;
  validPackets: number;
  invalidPackets: number;
  duplicatePackets: number;
  uniqueSymbols: number;
  neededSymbols: number | null;
  progress: number | null;
  filename: string | null;
  mimeType: string | null;
  originalSize: number | null;
  encrypted: boolean;
  failedDecryptions: number;
}

export interface ReceiveResult {
  file: ReconstructedFile;
}

type ReceiveMode = "gif" | "camera";

const emptyProgress: DecodeProgressState = {
  phase: "parsing",
  framesSeen: 0,
  validPackets: 0,
  invalidPackets: 0,
  duplicatePackets: 0,
  uniqueSymbols: 0,
  neededSymbols: null,
  progress: null,
  filename: null,
  mimeType: null,
  originalSize: null,
  encrypted: false,
  failedDecryptions: 0,
};

export function ReceivePage() {
  const [mode, setMode] = useState<ReceiveMode>("gif");
  const decodeRef = useRef<WorkerClient<DecodeRequest, DecodeResponse> | null>(null);

  useEffect(() => {
    const client = new WorkerClient<DecodeRequest, DecodeResponse>(
      () => new Worker(new URL("../../workers/decode.worker.ts", import.meta.url), { type: "module" })
    );
    decodeRef.current = client;
    return () => {
      client.terminate();
      decodeRef.current = null;
    };
  }, []);

  return (
    <section className="section" id="receive" aria-labelledby="receive-heading">
      <div className="card section-card">
      <header className="section-header">
        <h2 id="receive-heading" className="section-title">
          Receive a File
        </h2>
        <p className="page-sub">
          Reconstruct the original file from an animated QR GIF or by scanning with your camera.
          Every frame you capture is one more fountain packet — nothing needs to arrive in order.
        </p>
      </header>

      <div className="tabs" role="tablist" aria-label="Receive mode">
        <button
          role="tab"
          aria-selected={mode === "gif"}
          className={`tab${mode === "gif" ? " active" : ""}`}
          onClick={() => setMode("gif")}
        >
          <FileImage size={15} /> Import a GIF
        </button>
        <button
          role="tab"
          aria-selected={mode === "camera"}
          className={`tab${mode === "camera" ? " active" : ""}`}
          onClick={() => setMode("camera")}
        >
          <Camera size={15} /> Scan with camera
        </button>
      </div>

      {mode === "gif" ? (
        <GifImport decodeRef={decodeRef} />
      ) : (
        <CameraScan decodeRef={decodeRef} />
      )}
      </div>
    </section>
  );
}

interface DecodeRef {
  current: WorkerClient<DecodeRequest, DecodeResponse> | null;
}

function useReceiverStatus() {
  const [progress, setProgress] = useState<DecodeProgressState | null>(null);
  const [result, setResult] = useState<ReceiveResult | null>(null);
  const [incomplete, setIncomplete] = useState<DecodeProgressState | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [awaitingPassphrase, setAwaitingPassphrase] = useState<DecodeProgressState | null>(null);
  // The useState setters are stable across renders. Callbacks that run inside
  // effects (e.g. feedPacket) must depend on those setters directly — never on
  // this state bag, whose identity changes on every progress update.
  return {
    progress, setProgress,
    result, setResult,
    incomplete, setIncomplete,
    error, setError,
    busy, setBusy,
    awaitingPassphrase, setAwaitingPassphrase,
  };
}

/** Submitting the passphrase unlocks paused transfers (Rust-side outcome). */
interface PassphraseRpcResult {
  outcome: { unlocked: number; rejected: number };
  active: TransferStatus | null;
}

/**
 * Passphrase gate for encrypted transfers. Shown when the engine reports
 * `awaiting-passphrase`: the metadata envelope is understood, but nothing
 * (including the filename) can be read until a passphrase decrypts it.
 */
function PassphrasePanel({
  onSubmit,
  busy,
  rejected,
}: {
  onSubmit: (passphrase: string) => void;
  busy: boolean;
  rejected: boolean;
}) {
  const [value, setValue] = useState("");
  const formRef = useRef<HTMLFormElement>(null);
  return (
    <div className="card passphrase-card" role="status">
      <div className="passphrase-head">
        <KeyRound size={18} strokeWidth={1.6} />
        <div>
          <div className="notice-title">This transfer is encrypted</div>
          <div className="notice-detail">
            Enter the passphrase the sender gave you — it travels separately from
            the GIF. The filename and contents stay hidden until the passphrase
            decrypts the transfer.
          </div>
        </div>
      </div>
      <form
        ref={formRef}
        onSubmit={(e) => {
          e.preventDefault();
          if (value.length > 0 && !busy) {
            onSubmit(value);
          }
        }}
        className="passphrase-form"
      >
        <input
          type="password"
          value={value}
          onChange={(e) => setValue(e.target.value)}
          placeholder="Transfer passphrase"
          autoComplete="off"
          disabled={busy}
          aria-label="Transfer passphrase"
        />
        <button className="btn primary" type="submit" disabled={busy || value.length === 0}>
          Decrypt
        </button>
      </form>
      {rejected ? (
        <p className="hint passphrase-hint warn" role="alert">
          That passphrase did not decrypt the transfer. Check for typos, or the
          GIF may be corrupted.
        </p>
      ) : null}
    </div>
  );
}

function GifImport({ decodeRef }: { decodeRef: DecodeRef }) {
  const state = useReceiverStatus();
  const [rejectedAttempt, setRejectedAttempt] = useState(false);

  const cancel = useCallback(() => {
    // Fire-and-forget: the in-flight import loop checks the flag between frames.
    decodeRef.current?.send({ kind: "cancel-import" } as Omit<DecodeRequest, "id">);
  }, [decodeRef]);

  const importGif = useCallback(
    async (file: File) => {
      const client = decodeRef.current;
      if (!client) {
        return;
      }
      state.setBusy(true);
      state.setError(null);
      state.setResult(null);
      state.setIncomplete(null);
      state.setAwaitingPassphrase(null);
      setRejectedAttempt(false);
      state.setProgress({ ...emptyProgress });
      try {
        const data = await file.arrayBuffer();
        const res = await client.call<{ status: TransferStatus; file: ReconstructedFile | null }>(
          { kind: "import-gif", data } as Omit<DecodeRequest, "id">,
          (message) => {
            const m = message as DecodeProgressState & { kind: string };
            if (m.kind === "progress") {
              state.setProgress(m);
            }
          }
        );
        if (res.file) {
          state.setResult({ file: res.file });
        } else if (res.status.encrypted && res.status.phase === "awaiting-passphrase") {
          // Encrypted transfer paused at the passphrase gate.
          state.setProgress(null);
          state.setAwaitingPassphrase({
            ...emptyProgress,
            phase: "done",
            encrypted: true,
          });
        } else {
          // Not enough frames captured — report honestly, never fabricate.
          const s = res.status;
          state.setProgress(null);
          state.setIncomplete({
            ...emptyProgress,
            phase: "done",
            uniqueSymbols: s.uniqueSymbols,
            neededSymbols: s.neededSymbols,
            progress: s.progress,
            filename: s.filename,
            originalSize: s.originalSize,
            encrypted: s.encrypted,
            failedDecryptions: s.failedDecryptions,
          });
        }
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        if (message === "cancelled") {
          state.setProgress(null);
          state.setError(null);
        } else {
          state.setError(message);
        }
      } finally {
        state.setBusy(false);
      }
    },
    [decodeRef, state]
  );

  const submitPassphrase = useCallback(
    async (passphrase: string) => {
      const client = decodeRef.current;
      if (!client) {
        return;
      }
      state.setBusy(true);
      try {
        const res = await client.call<PassphraseRpcResult>(
          { kind: "set-passphrase", passphrase } as Omit<DecodeRequest, "id">
        );
        const status = res.active;
        if (res.outcome.rejected > 0 || !status || status.phase === "awaiting-passphrase") {
          setRejectedAttempt(true);
          return;
        }
        setRejectedAttempt(false);
        if (status.phase === "verified") {
          const fileRes = await client.call<{ file: ReconstructedFile }>(
            { kind: "take-file", transferId: status.transferId } as unknown as Omit<DecodeRequest, "id">
          );
          state.setAwaitingPassphrase(null);
          state.setResult({ file: fileRes.file });
          return;
        }
        // Unlocked but incomplete: show the honest shortfall.
        state.setAwaitingPassphrase(null);
        state.setIncomplete({
          ...emptyProgress,
          phase: "done",
          uniqueSymbols: status.uniqueSymbols,
          neededSymbols: status.neededSymbols,
          progress: status.progress,
          filename: status.filename,
          originalSize: status.originalSize,
          encrypted: status.encrypted,
          failedDecryptions: status.failedDecryptions,
        });
      } catch (err) {
        state.setError(err instanceof Error ? err.message : String(err));
      } finally {
        state.setBusy(false);
      }
    },
    [decodeRef, state]
  );

  return (
    <>
      {!state.result ? (
        <>
          <Dropzone
            gifOnly
            label="Drop the animated QR GIF here, or browse"
            hint="The GIF stays on this device — frames are decoded locally."
            onFile={(f) => void importGif(f)}
            onRejected={() =>
              state.setError(
                "That file is not a GIF. Import an animated QR GIF produced by Pigeon's Send tab."
              )
            }
            disabled={state.busy}
          />

          {state.error ? <ErrorDisplay title="Could not recover the file" detail={state.error} /> : null}

          {state.awaitingPassphrase ? (
            <PassphrasePanel onSubmit={(pw) => void submitPassphrase(pw)} busy={state.busy} rejected={rejectedAttempt} />
          ) : null}

          {state.incomplete ? <IncompletePanel status={state.incomplete} /> : null}

          {state.busy && state.progress ? (
            <div>
              <ReceiveProgress progress={state.progress} live={false} />
              <div className="actions">
                <button className="btn ghost" onClick={cancel}>
                  Cancel
                </button>
              </div>
            </div>
          ) : null}
        </>
      ) : (
        <ResultPanel
          result={state.result}
          onReset={() => {
            state.setResult(null);
            state.setIncomplete(null);
            state.setProgress(null);
            state.setAwaitingPassphrase(null);
            setRejectedAttempt(false);
          }}
        />
      )}
    </>
  );
}

function CameraScan({ decodeRef }: { decodeRef: DecodeRef }) {
  const videoRef = useRef<HTMLVideoElement>(null);
  const streamRef = useRef<MediaStream | null>(null);
  const rafRef = useRef<number>(0);
  const cameraRef = useRef<Worker | null>(null);
  const pendingRef = useRef(false);
  const activeRef = useRef(false);
  const state = useReceiverStatus();
  const [devices, setDevices] = useState<MediaDeviceInfo[]>([]);
  const [deviceId, setDeviceId] = useState<string>("");
  const [permissionError, setPermissionError] = useState<string | null>(null);
  const [scanning, setScanning] = useState(false);
  const [rejectedAttempt, setRejectedAttempt] = useState(false);

  const stop = useCallback(() => {
    activeRef.current = false;
    setScanning(false);
    cancelAnimationFrame(rafRef.current);
    streamRef.current?.getTracks().forEach((t) => t.stop());
    streamRef.current = null;
    if (videoRef.current) {
      videoRef.current.srcObject = null;
    }
  }, []);

  // Destructured setters are stable; `feedPacket` keeps a stable identity so
  // the camera-worker effect below runs once, not on every progress render.
  const { setProgress, setResult, setError, setIncomplete, setAwaitingPassphrase } = state;

  const feedPacket = useCallback(
    (bytes: Uint8Array) => {
      const client = decodeRef.current;
      if (!client) {
        return;
      }
      void client
        .call<{ last: { status: TransferStatus | null } | null; active: TransferStatus | null }>(
          { kind: "ingest-packets", packets: [bytes] } as Omit<DecodeRequest, "id">
        )
        .then((res) => {
          const status = res.active ?? res.last?.status ?? null;
          if (status) {
            setIncomplete(null);
            if (status.encrypted && status.phase === "awaiting-passphrase") {
              // Pause scanning feedback; the passphrase panel takes over.
              setAwaitingPassphrase({ ...emptyProgress, phase: "done", encrypted: true });
              return;
            }
            setAwaitingPassphrase(null);
            setProgress((prev) => ({
              ...emptyProgress,
              phase: "collecting",
              framesSeen: (prev?.framesSeen ?? 0) + 1,
              validPackets: (prev?.validPackets ?? 0) + 1,
              invalidPackets: prev?.invalidPackets ?? 0,
              duplicatePackets: prev?.duplicatePackets ?? 0,
              uniqueSymbols: status.uniqueSymbols,
              neededSymbols: status.neededSymbols,
              progress: status.progress,
              filename: status.filename,
              mimeType: status.mimeType,
              originalSize: status.originalSize,
              encrypted: status.encrypted,
              failedDecryptions: status.failedDecryptions,
            }));
            if (status.phase === "verified") {
              // Stop the camera first: the transfer is complete, the light
              // should go off without requiring another click.
              stop();
              void client
                .call<{ file: ReconstructedFile }>({
                  kind: "take-file",
                  transferId: status.transferId,
                } as unknown as Omit<DecodeRequest, "id">)
                .then((fileRes) => {
                  setResult({ file: fileRes.file });
                })
                .catch((err: unknown) =>
                  setError(err instanceof Error ? err.message : String(err))
                );
            }
          }
        })
        .catch((err: unknown) => {
          setError(err instanceof Error ? err.message : String(err));
        });
    },
    [decodeRef, setProgress, setResult, setError, setIncomplete, setAwaitingPassphrase, stop]
  );

  const submitPassphrase = useCallback(
    async (passphrase: string) => {
      const client = decodeRef.current;
      if (!client) {
        return;
      }
      state.setBusy(true);
      try {
        const res = await client.call<PassphraseRpcResult>(
          { kind: "set-passphrase", passphrase } as Omit<DecodeRequest, "id">
        );
        const status = res.active;
        if (res.outcome.rejected > 0 || !status || status.phase === "awaiting-passphrase") {
          setRejectedAttempt(true);
          return;
        }
        setRejectedAttempt(false);
        if (status.phase === "verified") {
          setAwaitingPassphrase(null);
          stop();
          const fileRes = await client.call<{ file: ReconstructedFile }>(
            { kind: "take-file", transferId: status.transferId } as unknown as Omit<DecodeRequest, "id">
          );
          state.setResult({ file: fileRes.file });
          return;
        }
        // Unlocked but still collecting: resume the live progress view.
        setAwaitingPassphrase(null);
        setProgress({
          ...emptyProgress,
          phase: "collecting",
          uniqueSymbols: status.uniqueSymbols,
          neededSymbols: status.neededSymbols,
          progress: status.progress,
          filename: status.filename,
          mimeType: status.mimeType,
          originalSize: status.originalSize,
          encrypted: status.encrypted,
          failedDecryptions: status.failedDecryptions,
        });
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err));
      } finally {
        state.setBusy(false);
      }
    },
    [decodeRef, state, setAwaitingPassphrase, setProgress, setError, stop]
  );

  const scanLoop = useCallback(() => {
    if (!activeRef.current) {
      return;
    }
    const video = videoRef.current;
    if (video && video.readyState >= 2 && !pendingRef.current) {
      pendingRef.current = true;
      createImageBitmap(video)
        .then((bitmap) => {
          cameraRef.current?.postMessage(
            { kind: "decode-frame", id: 0, bitmap, targetWidth: 720 },
            [bitmap]
          );
        })
        .catch(() => {
          pendingRef.current = false;
        });
    }
    rafRef.current = requestAnimationFrame(scanLoop);
  }, []);

  const start = useCallback(
    async (deviceIdToUse: string) => {
      setError(null);
      try {
        if (!navigator.mediaDevices?.getUserMedia) {
          throw new Error(
            "This browser does not expose the camera API. Camera scanning needs HTTPS or localhost."
          );
        }
        const constraints: MediaStreamConstraints = {
          video: deviceIdToUse
            ? { deviceId: { exact: deviceIdToUse } }
            : { facingMode: "environment" },
          audio: false,
        };
        const stream = await navigator.mediaDevices.getUserMedia(constraints);
        streamRef.current = stream;
        const video = videoRef.current;
        if (video) {
          video.srcObject = stream;
          await video.play();
        }
        const caps = navigator.mediaDevices.enumerateDevices
          ? await navigator.mediaDevices.enumerateDevices()
          : [];
        setDevices(caps.filter((d) => d.kind === "videoinput"));
        activeRef.current = true;
        setScanning(true);
        scanLoop();
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        if (message.toLowerCase().includes("permission") || message.toLowerCase().includes("denied")) {
          setPermissionError("Camera access was denied. Allow it in the browser's site settings, then try again.");
        } else {
          setPermissionError(message);
        }
      }
    },
    [scanLoop, setError]
  );

  // Camera worker lifecycle. `feedPacket` has a stable identity (its deps are
  // stable setters), so this effect creates the worker once — not on every
  // progress update.
  useEffect(() => {
    const worker = new Worker(new URL("../../workers/camera.worker.ts", import.meta.url), {
      type: "module",
    });
    worker.onmessage = (event: MessageEvent<{ kind: string; packets?: Uint8Array[] }>) => {
      pendingRef.current = false;
      if (event.data.kind === "frame-result" && event.data.packets) {
        for (const packet of event.data.packets) {
          feedPacket(packet);
        }
      }
    };
    cameraRef.current = worker;
    return () => {
      worker.terminate();
      cameraRef.current = null;
      activeRef.current = false;
      cancelAnimationFrame(rafRef.current);
    };
  }, [feedPacket]);

  useEffect(() => () => stop(), [stop]);

  return (
    <>
      <div className="card camera-card">
        <video ref={videoRef} className="camera-view" muted playsInline aria-label="Camera preview" />
        <div className="camera-controls">
          {!scanning ? (
            <button className="btn primary" onClick={() => void start(deviceId)}>
              <Camera size={16} /> Start camera
            </button>
          ) : (
            <button className="btn ghost" onClick={stop}>
              <CameraOff size={16} /> Stop camera
            </button>
          )}
          {devices.length > 1 ? (
            <label className="field inline">
              <span>Camera</span>
              <select
                value={deviceId}
                onChange={(e) => {
                  const next = e.target.value;
                  setDeviceId(next);
                  if (activeRef.current) {
                    stop();
                    // Restart with the newly chosen device, not a stale closure.
                    setTimeout(() => void start(next), 150);
                  }
                }}
              >
                <option value="">Default (rear)</option>
                {devices.map((d, i) => (
                  <option key={d.deviceId} value={d.deviceId}>
                    {d.label || `Camera ${i + 1}`}
                  </option>
                ))}
              </select>
            </label>
          ) : null}
        </div>
        {permissionError ? (
          <ErrorDisplay
            tone="warning"
            title="Camera unavailable"
            detail={`${permissionError} Camera access requires HTTPS or localhost.`}
          />
        ) : null}
        <p className="hint">
          Hold the receiving camera steady at roughly arm's length from the transmitting screen.
          Frames arrive in any order; duplicates are harmless.
        </p>
      </div>

      {state.error ? <ErrorDisplay title="Recovery failed" detail={state.error} /> : null}

      {state.result ? (
        <ResultPanel
          result={state.result}
          onReset={() => {
            state.setResult(null);
            state.setProgress(null);
            state.setAwaitingPassphrase(null);
            setRejectedAttempt(false);
            void decodeRef.current?.call({ kind: "reset" } as Omit<DecodeRequest, "id">);
          }}
        />
      ) : state.awaitingPassphrase ? (
        <PassphrasePanel onSubmit={(pw) => void submitPassphrase(pw)} busy={state.busy} rejected={rejectedAttempt} />
      ) : state.progress ? (
        <ReceiveProgress progress={state.progress} live={scanning} />
      ) : null}
    </>
  );
}

function ReceiveProgress({
  progress,
  live,
}: {
  progress: DecodeProgressState;
  live: boolean;
}) {
  return (
    <div className="card" aria-live="polite">
      <ProgressBar
        value={progress.progress}
        label={`${humanPhase(progress.phase)}${progress.filename ? ` — ${progress.filename}` : ""}`}
        detail={
          progress.neededSymbols != null
            ? `${progress.uniqueSymbols} / ${progress.neededSymbols} unique packets${live ? " · capture more frames to finish" : ""}`
            : `${progress.uniqueSymbols} unique packets · waiting for a metadata frame`
        }
      />
      <div className="stats-grid compact">
        <Stat label="Frames seen" value={String(progress.framesSeen)} mono />
        <Stat label="Valid packets" value={String(progress.validPackets)} mono />
        <Stat label="Invalid" value={String(progress.invalidPackets)} mono />
        <Stat label="Duplicates" value={String(progress.duplicatePackets)} mono />
        {progress.encrypted ? <Stat label="Mode" value="Encrypted" /> : null}
        {progress.failedDecryptions > 0 ? (
          <Stat label="Rejected packets" value={String(progress.failedDecryptions)} mono />
        ) : null}
      </div>
      {progress.encrypted && progress.failedDecryptions > 0 ? (
        <p className="hint passphrase-hint warn">
          Packets are failing to decrypt — the passphrase may be wrong for this
          transfer.
        </p>
      ) : null}
    </div>
  );
}

function IncompletePanel({ status }: { status: DecodeProgressState }) {
  return (
    <div className="notice notice-warning" role="status">
      <div>
        <div className="notice-title">
          Not enough frames captured yet — nothing was fabricated
        </div>
        <div className="notice-detail">
          {status.neededSymbols != null
            ? `Captured ${status.uniqueSymbols} of ${status.neededSymbols} required unique packets (${Math.round((status.progress ?? 0) * 100)}%). Re-import the GIF, use a copy with more frames intact, or scan the QR frames with your camera for the remainder.`
            : "No metadata frame was decoded, so the transfer parameters are unknown. The GIF may have been altered by a messaging platform — send it as a document/file."}
        </div>
      </div>
    </div>
  );
}

function ResultPanel({ result, onReset }: { result: ReceiveResult; onReset: () => void }) {
  const f = result.file;
  return (
    <div className="card result-card" aria-label="Recovered file">
      <div className="result-head">
        <div>
          <h2>File recovered &amp; verified</h2>
          <p className="page-sub">
            Reconstructed from {formatBytes(f.originalSize)} of original data; SHA-256 matches
            the sender's manifest.
          </p>
        </div>
        <button className="btn ghost small" onClick={onReset}>
          <RefreshCw size={14} /> Receive another
        </button>
      </div>
      <div className="stats-grid">
        <Stat label="Filename" value={f.filename} />
        <Stat label="Size" value={formatBytes(f.originalSize)} />
        <Stat label="Type" value={f.mimeType ?? "unknown"} />
        <Stat label="SHA-256" value={`${f.sha256.slice(0, 16)}…`} mono />
      </div>
      <p className="hint mono selectable">{f.sha256}</p>
      <div className="actions">
        <button
          className="btn primary"
          onClick={() => downloadBlob(f.data, f.filename, f.mimeType ?? "application/octet-stream")}
        >
          <Download size={16} /> Download {f.filename}
        </button>
      </div>
    </div>
  );
}
