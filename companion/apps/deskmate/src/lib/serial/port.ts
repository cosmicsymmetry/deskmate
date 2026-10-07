import {
  Deframer,
  type Decoded,
  type DeviceError,
  encodeNetworkConfig,
  encodeStatusRequest,
  MessageType,
  type NetworkConfig,
  type StatusResponse,
} from "./codec";

export interface PanelPort {
  open(): Promise<void>;
  write(bytes: Uint8Array): Promise<void>;
  readable(): AsyncIterable<Uint8Array>;
  close(): Promise<void>;
  /**
   * Restarts the board. NetworkConfig is persisted but only read at boot, so a
   * write means nothing until this (or an unplug) happens.
   */
  restart(): Promise<void>;
  onDisconnect(listener: () => void): void;
}

const PANEL_USB_FILTER = { usbVendorId: 0x303a, usbProductId: 0x1001 } as const;

type BrowserSerialPort = EventTarget & {
  readable: ReadableStream<Uint8Array> | null;
  writable: WritableStream<Uint8Array> | null;
  open(options: { baudRate: number }): Promise<void>;
  close(): Promise<void>;
  setSignals(signals: { dataTerminalReady?: boolean; requestToSend?: boolean }): Promise<void>;
};

type BrowserSerial = EventTarget & {
  requestPort(options: { filters: Array<typeof PANEL_USB_FILTER> }): Promise<BrowserSerialPort>;
};

type BrowserSerialDisconnectEvent = Event & { port?: BrowserSerialPort };

type NavigatorWithSerial = Navigator & { serial: BrowserSerial };

export class PanelDeviceError extends Error {
  constructor(readonly deviceError: DeviceError) {
    super(deviceError.diagnostic);
    this.name = "PanelDeviceError";
  }
}

export class PanelTimeoutError extends Error {
  constructor() {
    super("The panel did not respond in time.");
    this.name = "PanelTimeoutError";
  }
}

export class PanelDisconnectedError extends Error {
  constructor() {
    super("The panel was disconnected.");
    this.name = "PanelDisconnectedError";
  }
}

export function serialSupported(): boolean {
  return typeof navigator !== "undefined" && "serial" in navigator;
}

export class WebSerialPort implements PanelPort {
  private readonly disconnectListeners = new Set<() => void>();
  private opened = false;

  constructor(
    private readonly port: BrowserSerialPort,
    serial: BrowserSerial,
  ) {
    const disconnected = (event: Event) => {
      const serialEvent = event as BrowserSerialDisconnectEvent;
      if (event.target === this.port || serialEvent.port === this.port) this.signalDisconnect();
    };
    serial.addEventListener("disconnect", disconnected);
    port.addEventListener("disconnect", () => this.signalDisconnect());
  }

  async open(): Promise<void> {
    if (this.opened) return;
    await this.port.open({ baudRate: 115_200 });
    this.opened = true;
  }

  async write(bytes: Uint8Array): Promise<void> {
    const stream = this.port.writable;
    if (stream === null) throw new PanelDisconnectedError();
    const writer = stream.getWriter();
    try {
      await writer.write(bytes);
    } catch (error) {
      this.signalDisconnect();
      throw error;
    } finally {
      writer.releaseLock();
    }
  }

  async *readable(): AsyncIterable<Uint8Array> {
    const stream = this.port.readable;
    if (stream === null) throw new PanelDisconnectedError();
    const reader = stream.getReader();
    try {
      while (true) {
        const { value, done } = await reader.read();
        if (done) {
          this.signalDisconnect();
          return;
        }
        if (value !== undefined) yield value;
      }
    } catch (error) {
      this.signalDisconnect();
      throw error;
    } finally {
      reader.releaseLock();
    }
  }

  async close(): Promise<void> {
    if (!this.opened) return;
    this.opened = false;
    await this.port.close();
  }

  async restart(): Promise<void> {
    if (!this.opened) return;
    // The ESP32-S3's USB-Serial-JTAG resets the chip when RTS is asserted with
    // DTR low -- the same sequence esptool uses for a hard reset.
    await this.port.setSignals({ dataTerminalReady: false, requestToSend: true });
    await new Promise((resolve) => setTimeout(resolve, 100));
    await this.port.setSignals({ requestToSend: false });
  }

  onDisconnect(listener: () => void): void {
    this.disconnectListeners.add(listener);
  }

  private signalDisconnect(): void {
    if (!this.opened) return;
    this.opened = false;
    for (const listener of this.disconnectListeners) listener();
  }
}

export async function requestPanelPort(): Promise<PanelPort> {
  if (!serialSupported()) throw new Error("Web Serial is not supported in this browser.");
  const serial = (navigator as NavigatorWithSerial).serial;
  const port = await serial.requestPort({ filters: [PANEL_USB_FILTER] });
  return new WebSerialPort(port, serial);
}

type PendingRequest<T = unknown> = {
  requestId: number;
  accept(decoded: Decoded): T | undefined;
  resolve(value: T): void;
  reject(error: Error): void;
  timeout: ReturnType<typeof setTimeout>;
};

export class PanelLink {
  private readonly deframer = new Deframer();
  private nextRequestId = 1;
  private pending: PendingRequest | undefined;
  private readerTask: Promise<void> | undefined;

  constructor(private readonly port: PanelPort) {}

  status(): Promise<StatusResponse> {
    return this.request(encodeStatusRequest, (decoded) => {
      if (decoded.type === "status") return decoded.status;
      if (decoded.type === "error") throw new PanelDeviceError(decoded.error);
      return undefined;
    });
  }

  async networkConfig(config: NetworkConfig): Promise<void> {
    await this.request(
      (requestId) => encodeNetworkConfig(requestId, config),
      (decoded) => {
        if (decoded.type === "error") throw new PanelDeviceError(decoded.error);
        if (decoded.type === "ack" && decoded.ack.acknowledgedType === MessageType.NetworkConfig) {
          return true;
        }
        return undefined;
      },
    );
  }

  private async request<T>(
    encode: (requestId: number) => Uint8Array,
    accept: (decoded: Decoded) => T | undefined,
  ): Promise<T> {
    if (this.pending !== undefined) throw new Error("A panel request is already in progress.");
    const requestId = this.takeRequestId();
    let resolveResponse!: (value: T) => void;
    let rejectResponse!: (error: Error) => void;
    const response = new Promise<T>((resolve, reject) => {
      resolveResponse = resolve;
      rejectResponse = reject;
    });
    const timeout = setTimeout(() => {
      this.rejectPending(requestId, new PanelTimeoutError());
    }, 2_000);
    this.pending = {
      requestId,
      accept,
      resolve: resolveResponse,
      reject: rejectResponse,
      timeout,
    };

    try {
      await this.port.write(encode(requestId));
      this.startReader();
    } catch (error) {
      this.rejectPending(
        requestId,
        error instanceof Error ? error : new Error("Could not write to the panel."),
      );
    }
    return response;
  }

  private takeRequestId(): number {
    const requestId = this.nextRequestId;
    this.nextRequestId = requestId === 0xffffffff ? 1 : requestId + 1;
    return requestId;
  }

  private startReader(): void {
    if (this.readerTask !== undefined) return;
    const task = this.readLoop();
    this.readerTask = task;
    void task.finally(() => {
      if (this.readerTask === task) this.readerTask = undefined;
    });
  }

  private async readLoop(): Promise<void> {
    try {
      for await (const chunk of this.port.readable()) {
        for (const decoded of this.deframer.push(chunk)) {
          if (decoded.type === "invalid") continue;
          if (decoded.type === "other" && decoded.messageType === 12 && decoded.requestId === 0) {
            continue;
          }
          const pending = this.pending;
          if (pending === undefined || decoded.requestId !== pending.requestId) continue;
          try {
            const accepted = pending.accept(decoded);
            if (accepted !== undefined) this.resolvePending(pending.requestId, accepted);
          } catch (error) {
            this.rejectPending(
              pending.requestId,
              error instanceof Error ? error : new Error("The panel returned an error."),
            );
          }
        }
      }
      this.rejectCurrent(new PanelDisconnectedError());
    } catch (error) {
      this.rejectCurrent(error instanceof Error ? error : new PanelDisconnectedError());
    }
  }

  private resolvePending<T>(requestId: number, value: T): void {
    const pending = this.pending;
    if (pending === undefined || pending.requestId !== requestId) return;
    this.pending = undefined;
    clearTimeout(pending.timeout);
    pending.resolve(value);
  }

  private rejectPending(requestId: number, error: Error): void {
    const pending = this.pending;
    if (pending === undefined || pending.requestId !== requestId) return;
    this.pending = undefined;
    clearTimeout(pending.timeout);
    pending.reject(error);
  }

  private rejectCurrent(error: Error): void {
    const pending = this.pending;
    if (pending !== undefined) this.rejectPending(pending.requestId, error);
  }
}
