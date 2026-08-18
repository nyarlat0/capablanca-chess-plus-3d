/* TeraStockfish runs in its own classic worker so a large-board search never
 * blocks rendering, input, or piece animations on the browser's main thread. */
importScripts("terastockfish.js");

let engine = null;
let hashMegabytes = 16;
let positionFen = null;

const initialized = wasm_bindgen(
    new URL("terastockfish_bg.wasm", self.location.href)
).then(() => {
    engine = new wasm_bindgen.WebEngine(hashMegabytes);
});

function fail(error) {
    const message = error instanceof Error ? error.message : String(error);
    self.postMessage(`__tera_error__ ${message}`);
}

function valueAfter(command, marker) {
    const index = command.toLowerCase().indexOf(marker.toLowerCase());
    return index < 0 ? "" : command.slice(index + marker.length).trim();
}

async function handleCommand(command) {
    if (command === "quit") {
        self.close();
        return;
    }
    if (command === "uci") {
        self.postMessage("id name TeraStockfish Web");
        self.postMessage("option name Hash type spin default 16 min 1 max 256");
        self.postMessage("option name Threads type spin default 1 min 1 max 1");
        self.postMessage("option name UCI_Variant type combo default terachessii var terachessii");
        self.postMessage("uciok");
        return;
    }
    if (command === "isready") {
        await initialized;
        self.postMessage("readyok");
        return;
    }
    if (command === "ucinewgame") {
        await initialized;
        engine.clear_hash();
        positionFen = null;
        return;
    }
    if (command.startsWith("setoption name Hash value ")) {
        const requested = Number(valueAfter(command, "value"));
        if (!Number.isInteger(requested) || requested < 1 || requested > 256) {
            throw new Error("Hash must be an integer between 1 and 256 MiB");
        }
        await initialized;
        if (requested !== hashMegabytes) {
            hashMegabytes = requested;
            engine.free();
            engine = new wasm_bindgen.WebEngine(hashMegabytes);
        }
        return;
    }
    if (command.startsWith("setoption name Threads value ")) {
        return;
    }
    if (command.startsWith("setoption name UCI_Variant value ")) {
        const variant = valueAfter(command, "value").toLowerCase();
        if (variant !== "terachessii" && variant !== "terachess ii") {
            throw new Error(`unsupported TeraStockfish variant: ${variant}`);
        }
        return;
    }
    if (command.startsWith("position fen ")) {
        positionFen = command.slice("position fen ".length);
        return;
    }
    if (command.startsWith("go nodes ")) {
        const nodes = Number(command.slice("go nodes ".length));
        if (!Number.isSafeInteger(nodes) || nodes < 1) {
            throw new Error("node budget must be a positive safe integer");
        }
        if (positionFen === null) {
            throw new Error("position must be set before search");
        }
        await initialized;
        const response = engine.analyze(positionFen, BigInt(nodes));
        for (const line of response.split("\n")) {
            if (line.length > 0) self.postMessage(line);
        }
        return;
    }
    // A running WebAssembly call cannot observe queued worker messages.
    // The Bevy side cancels by terminating this worker instead.
    if (command === "stop") return;
    throw new Error(`unknown TeraStockfish command: ${command}`);
}

// MessageEvent dispatch does not await an async handler. Keep the UCI stream
// ordered explicitly, especially while the WebAssembly module is initializing.
let commandQueue = Promise.resolve();
self.onmessage = (event) => {
    const command = String(event.data).trim();
    commandQueue = commandQueue.then(() => handleCommand(command)).catch(fail);
};

initialized.catch(fail);
