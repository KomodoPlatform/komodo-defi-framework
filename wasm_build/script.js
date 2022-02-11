// Use ES module import syntax to import functionality from the module
// that we have compiled.
//
// Note that the `default` import is an initialization function which
// will "boot" the module and make it ready to use. Currently browsers
// don't support natively imported WebAssembly as an ES module, but
// eventually the manual initialization won't be required!
import init, {
    mm2_main,
    mm2_main_status,
    mm2_rpc,
    mm2_version,
    LogLevel,
    Mm2MainErr,
    MainStatus,
    Mm2RpcErr
} from "./deps/pkg/mm2.js";

const LOG_LEVEL = LogLevel.Debug;

// Loads the wasm file, so we use the
// default export to inform it where the wasm file is located on the
// server, and then we wait on the returned promise to wait for the
// wasm to be loaded.
async function init_wasm() {
    try {
        await init();
    } catch (e) {
        alert(`Oops: ${e}`);
    }
}

async function run_mm2(params) {
    // run an MM2 instance
    try {
        const version = mm2_version();
        console.info(`run_mm2() version=${version.result} datetime=${version.datetime}`);

        mm2_main(params, handle_log);
    } catch (e) {
        switch (e) {
            case Mm2MainErr.AlreadyRuns:
                alert("MM2 already runs, please wait...");
                return;
            case Mm2MainErr.InvalidParams:
                alert("Invalid config");
                return;
            case Mm2MainErr.NoCoinsInConf:
                alert("No 'coins' field in config");
                return;
            default:
                alert(`Oops: ${e}`);
                return;
        }
    }
}

async function rpc_request(request_js) {
    try {
        const response = await mm2_rpc(request_js);
        console.log(response);
    } catch (e) {
        switch (e) {
            case Mm2RpcErr.NotRunning:
                alert("MM2 is not running yet");
                break;
            case Mm2RpcErr.InvalidPayload:
                alert(`Invalid payload: ${request_js}`);
                break;
            case Mm2RpcErr.InternalError:
                alert(`An MM2 internal error`);
                break;
            default:
                alert(`Unexpected error: ${e}`);
                break;
        }
    }
}

function handle_log(level, line) {
    switch (level) {
        case LogLevel.Off:
            break;
        case LogLevel.Error:
            console.error(line);
            break;
        case LogLevel.Warn:
            console.warn(line);
            break;
        case LogLevel.Info:
            console.info(line);
            break;
        case LogLevel.Debug:
            console.log(line);
            break;
        case LogLevel.Trace:
        default:
            // The console.trace method outputs some extra trace from the generated JS glue code which we don't want.
            console.debug(line);
            break;
    }
}

function spawn_mm2_status_checking() {
    setInterval(function () {
        const run_button = document.getElementById("wid_run_mm2_button");
        const rpc_button = document.getElementById("wid_mm2_rpc_button");

        const status = mm2_main_status();
        switch (status) {
            case MainStatus.NotRunning:
            case MainStatus.NoContext:
            case MainStatus.NoRpc:
                rpc_button.disabled = true;
                run_button.disabled = false;
                break;
            case MainStatus.RpcIsUp:
                rpc_button.disabled = false;
                run_button.disabled = true;
                break;
            default:
                throw new Error(`Expected MainStatus, found: ${status}`);
        }
    }, 100)
}

// The script starts here

init_wasm().then(function () {
    spawn_mm2_status_checking();
    const run_mm2_button = document.getElementById("wid_run_mm2_button");
    run_mm2_button.addEventListener('click', async () => {
        const conf = document.getElementById("wid_conf_input").value;
        let params;
        try {
            const conf_js = JSON.parse(conf);
            params = {
                conf: conf_js,
                log_level: LOG_LEVEL,
            };
        } catch (e) {
            alert(`Expected config in JSON, found '${conf}'\nError : ${e}`);
            return;
        }

        await run_mm2(params);
    });

    const rpc_request_button = document.getElementById("wid_mm2_rpc_button");
    rpc_request_button.addEventListener('click', async () => {
        const request_payload = document.getElementById("wid_rpc_input").value;
        let request_js;
        try {
            request_js = JSON.parse(request_payload);
        } catch (e) {
            alert(`Expected request in JSON, found '${request_payload}'\nError : ${e}`);
            return;
        }

        await rpc_request(request_js);
    });
});
