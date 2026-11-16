// Typed promise-based RPC over Web Worker postMessage with progress support.

export interface PendingCall {
  resolve: (value: unknown, transfer: ArrayBuffer[]) => void;
  reject: (err: Error) => void;
  onProgress?: (payload: unknown) => void;
}

export class WorkerClient<TRequest, TResponse extends { id: number; kind: string }> {
  private worker: Worker;
  private pending = new Map<number, PendingCall>();
  private nextId = 1;

  constructor(createWorker: () => Worker) {
    // Callers must construct the worker with an inline
    // `new Worker(new URL("./x.ts", import.meta.url), { type: "module" })`
    // expression so the bundler can detect and transform the import.
    this.worker = createWorker();
    this.worker.onmessage = (event: MessageEvent<TResponse>) => {
      const message = event.data;
      const call = this.pending.get(message.id);
      if (!call) {
        return;
      }
      if (message.kind === "progress") {
        call.onProgress?.(message);
        return;
      }
      if (message.kind === "err") {
        this.pending.delete(message.id);
        call.reject(new Error((message as unknown as { message: string }).message));
        return;
      }
      if (message.kind === "ok") {
        this.pending.delete(message.id);
        const transfer = (message as unknown as { transfer?: ArrayBuffer[] }).transfer ?? [];
        call.resolve((message as unknown as { result: unknown }).result, transfer);
        return;
      }
      // Extra kinds (e.g. live-frame) are handled via onProgress.
      call.onProgress?.(message);
    };
    this.worker.onerror = (event) => {
      const err = new Error(event.message || "worker crashed");
      for (const call of this.pending.values()) {
        call.reject(err);
      }
      this.pending.clear();
    };
  }

  /** Sends a request; resolves with the final ok result. */
  call<TResult>(payload: Omit<TRequest, "id">, onProgress?: (payload: unknown) => void): Promise<TResult> {
    const id = this.nextId++;
    return new Promise<TResult>((resolve, reject) => {
      this.pending.set(id, {
        resolve: resolve as (value: unknown, transfer: ArrayBuffer[]) => void,
        reject,
        onProgress,
      });
      this.worker.postMessage({ ...payload, id });
    });
  }

  /** Fire-and-forget (e.g. cancellation). */
  send(payload: Omit<TRequest, "id">): void {
    this.worker.postMessage({ ...payload, id: this.nextId++ });
  }

  terminate(): void {
    this.worker.terminate();
  }
}
