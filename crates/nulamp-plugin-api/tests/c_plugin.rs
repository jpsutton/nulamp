//! Builds a small plugin written in C against `include/nulamp_plugin.h` and loads it, so the
//! header and the Rust definitions are checked against each other. Skipped when there is no C
//! compiler (`cc`).
#![cfg(unix)]

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

use nulamp_audio::chain::Stage;
use nulamp_core::command::{Command as PlayerCommand, PlayerStatus};
use nulamp_plugin_api::{Host, HostCommand, HostServices, LoadOptions, PluginEvent, load_plugins};

const SOURCE: &str = r#"
#include <stddef.h>
#include <stdio.h>
#include <string.h>
#include "nulamp_plugin.h"

static const NulampHost *g_host;
static uint64_t g_ids[2];

static int32_t init(void *user, const NulampHost *host, uint64_t id) {
    g_host = host;
    g_ids[(size_t)user] = id;
    return NULAMP_OK;
}
static void quit(void *user) { (void)user; }

typedef struct { uint32_t channels; } Inst;
static Inst g_inst;
static void *create(void *user) { (void)user; return &g_inst; }
static void destroy(void *inst) { (void)inst; }
static int32_t configure(void *inst, uint32_t rate, uint32_t channels) {
    (void)rate;
    ((Inst *)inst)->channels = channels;
    return NULAMP_OK;
}
/* Swaps the first two channels and drops every second frame. */
static int64_t process(void *inst, const float *in, uint32_t frames, float *out,
                       uint32_t max_out) {
    uint32_t ch = ((Inst *)inst)->channels, n = 0;
    if (max_out < 2 * frames) return NULAMP_ERR_INVALID;
    for (uint32_t f = 0; f < frames; f += 2, n++) {
        for (uint32_t c = 0; c < ch; c++) out[n * ch + c] = in[f * ch + c];
        if (ch >= 2) { out[n * ch] = in[f * ch + 1]; out[n * ch + 1] = in[f * ch]; }
    }
    return n;
}
static uint32_t latency(void *inst) { (void)inst; return 3; }

static void on_event(void *user, const NulampEvent *ev) {
    (void)user;
    char msg[160];
    char title[64];
    NulampStatus st;
    memset(&st, 0, sizeof st);
    st.abi_version = NULAMP_ABI_VERSION;
    st.struct_size = sizeof st;
    g_host->get_status(g_host, &st);
    g_host->get_title(g_host, title, sizeof title);
    snprintf(msg, sizeof msg, "%u %lld %s vol=%d title=%s", ev->kind, (long long)ev->value,
             ev->text, st.volume, title);
    g_host->log(g_host, g_ids[1], NULAMP_LOG_INFO, msg);
    if (ev->kind == NULAMP_EVENT_PAUSED) g_host->send_command(g_host, NULAMP_CMD_SET_VOLUME, 42);
    g_host->settings_set(g_host, g_ids[1], "seen", ev->text);
}

static const NulampDspApi dsp_api = {
    NULAMP_ABI_VERSION, sizeof(NulampDspApi), create, destroy, configure, process, latency, NULL,
};
static const NulampGeneralApi general_api = {
    NULAMP_ABI_VERSION, sizeof(NulampGeneralApi), on_event,
};
static const NulampModule dsp = {
    NULAMP_ABI_VERSION, sizeof(NulampModule), NULAMP_KIND_DSP, 0, &dsp_api,
    "Swap and skip", "1", "C test effect", (void *)0, init, quit, NULL, NULL,
};
static const NulampModule general = {
    NULAMP_ABI_VERSION, sizeof(NulampModule), NULAMP_KIND_GENERAL, 0, &general_api,
    "C logger", "1", "C test logger", (void *)1, init, quit, NULL, NULL,
};
static const NulampModule *const modules[] = { &dsp, &general };
static const NulampPluginDesc desc = {
    NULAMP_ABI_VERSION, sizeof(NulampPluginDesc), "C plugin", "1", "", 2, 0, modules,
};

NULAMP_EXPORT const NulampPluginDesc *nulamp_plugin_entry(const NulampHost *host) {
    if (NULAMP_ABI_MAJOR_OF(host->abi_version) != NULAMP_ABI_MAJOR) return NULL;
    return &desc;
}
"#;

#[derive(Default)]
struct Services {
    commands: Mutex<Vec<HostCommand>>,
    logs: Mutex<Vec<String>>,
    settings: Mutex<Vec<(String, String, String)>>,
}

impl HostServices for Services {
    fn command(&self, command: HostCommand) {
        self.commands.lock().unwrap().push(command);
    }
    fn status(&self) -> PlayerStatus {
        PlayerStatus {
            volume: 99,
            title: "Song".into(),
            ..PlayerStatus::default()
        }
    }
    fn log(&self, module: &str, _level: log::Level, message: &str) {
        self.logs
            .lock()
            .unwrap()
            .push(format!("{module}: {message}"));
    }
    fn setting(&self, _section: &str, _key: &str) -> Option<String> {
        None
    }
    fn set_setting(&self, section: &str, key: &str, value: Option<&str>) {
        self.settings.lock().unwrap().push((
            section.into(),
            key.into(),
            value.unwrap_or_default().into(),
        ));
    }
    fn post_to_ui(&self, task: Box<dyn FnOnce() + Send>) {
        task();
    }
}

#[test]
fn c_plugin() {
    let dir = std::env::temp_dir().join(format!("nulamp-c-plugin-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("cplug.c");
    std::fs::write(&src, SOURCE).unwrap();
    let include = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../include");
    let lib = dir.join(format!("cplug.{}", std::env::consts::DLL_EXTENSION));
    let shared = if cfg!(target_os = "macos") {
        "-dynamiclib"
    } else {
        "-shared"
    };
    let built = Command::new("cc")
        .args([
            "-std=c99", "-Wall", "-Wextra", "-Werror", "-fPIC", shared, "-o",
        ])
        .arg(&lib)
        .arg("-I")
        .arg(&include)
        .arg(&src)
        .status();
    match built {
        Ok(s) => assert!(s.success(), "the C plugin does not compile"),
        Err(e) => {
            println!("skipped: no C compiler ({e})");
            std::fs::remove_dir_all(&dir).unwrap();
            return;
        }
    }
    std::fs::remove_file(&src).unwrap();

    let services = Arc::new(Services::default());
    let host = Host::new(services.clone());
    let opts = LoadOptions {
        system_dir: Some(dir.clone()),
        ..LoadOptions::default()
    };
    // SAFETY: the directory holds only the plugin built above, which follows the ABI.
    let plugins = unsafe { load_plugins(&opts, &host) };
    assert!(plugins.errors.is_empty(), "{:?}", plugins.errors);
    assert_eq!(plugins.dsps.len(), 1);
    assert_eq!(plugins.generals.len(), 1);
    assert_eq!(plugins.dsps[0].module().info().description, "C test effect");

    let mut d = plugins.dsps[0].instantiate().unwrap();
    assert_eq!(d.latency_frames(), 3);
    d.configure(44_100, 2);
    let mut v = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    d.process_resizing(&mut v);
    assert_eq!(v, [2.0, 1.0, 6.0, 5.0]);
    drop(d);

    plugins.broadcast(&PluginEvent::SkinChanged("Bento".into()));
    plugins.broadcast(&PluginEvent::Paused);
    assert_eq!(
        *services.logs.lock().unwrap(),
        [
            "C plugin/C logger: 7 0 Bento vol=99 title=Song",
            "C plugin/C logger: 8 0  vol=99 title=Song",
        ]
    );
    assert_eq!(
        *services.commands.lock().unwrap(),
        [HostCommand::Player(PlayerCommand::SetVolume(42))]
    );
    assert_eq!(
        services.settings.lock().unwrap()[0],
        (
            "plugin:cplug:C_logger".into(),
            "seen".into(),
            "Bento".into()
        )
    );
    drop(plugins);
    std::fs::remove_dir_all(&dir).unwrap();
}
