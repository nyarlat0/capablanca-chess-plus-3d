/* Keep Fairy-Stockfish and its pthread helper off the Bevy/WASM main thread. */
const assetBaseUrl = new URL(".", self.location.href);
const stockfishScriptUrl = new URL("stockfish.js", assetBaseUrl).href;
const variantsUrl = new URL("variants.ini", assetBaseUrl).href;

importScripts(stockfishScriptUrl);

const engineReady = Promise.resolve().then(() => {
    if (!self.crossOriginIsolated || typeof SharedArrayBuffer === "undefined") {
        throw new Error(
            "Fairy-Stockfish requires cross-origin isolation; enable COOP and COEP headers",
        );
    }

    return Promise.all([
        Stockfish({
            // Emscripten cannot infer its own URL after stockfish.js has been
            // loaded with importScripts(). Without this value it sends
            // `undefined` to the pthread helper as urlOrBlob, which makes
            // URL.createObjectURL() fail in Chromium-based browsers.
            mainScriptUrlOrBlob: stockfishScriptUrl,
            locateFile: (path) => new URL(path, assetBaseUrl).href,
        }),
        fetch(variantsUrl).then((response) => {
            if (!response.ok) {
                throw new Error(`Unable to load variants.ini: HTTP ${response.status}`);
            }
            return response.arrayBuffer();
        }),
    ]).then(([engine, variants]) => {
        engine.FS.writeFile("/variants.ini", new Uint8Array(variants));
        engine.addMessageListener((line) => self.postMessage(String(line)));
        engine.postMessage("setoption name VariantPath value /variants.ini");
        return engine;
    });
});

let errorReported = false;
function describeError(error) {
    if (error instanceof Error) {
        return error.stack || error.message;
    }
    return error == null ? "Unknown Fairy-Stockfish initialization error" : String(error);
}

function reportError(error) {
    if (!errorReported) {
        errorReported = true;
        self.postMessage(`__fairy_error__ ${describeError(error)}`);
    }
}

self.onmessage = (event) => {
    engineReady
        .then((engine) => engine.postMessage(String(event.data)))
        .catch(reportError);
};

engineReady.catch(reportError);
