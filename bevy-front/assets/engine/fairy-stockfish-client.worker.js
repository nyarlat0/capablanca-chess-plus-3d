/* Keep Fairy-Stockfish and its pthread helper off the Bevy/WASM main thread. */
importScripts("stockfish.js");

const engineReady = Promise.all([
    Stockfish(),
    fetch(new URL("variants.ini", self.location.href)).then((response) => {
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

let errorReported = false;
function reportError(error) {
    if (!errorReported) {
        errorReported = true;
        self.postMessage(`__fairy_error__ ${error}`);
    }
}

self.onmessage = (event) => {
    engineReady
        .then((engine) => engine.postMessage(String(event.data)))
        .catch(reportError);
};

engineReady.catch(reportError);
