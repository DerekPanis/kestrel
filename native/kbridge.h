// SPDX-License-Identifier: AGPL-3.0-only
//
// kbridge — a small, stable C API over chiaki-ng's libchiaki.
//
// libchiaki's public structs (ChiakiSession etc.) are large, change between
// releases and pull in platform socket headers. Rust only ever sees the flat,
// opaque API in this file, so the FFI surface stays small and layout-safe.
// Every struct here is mirrored 1:1 in crates/kestrel-core/src/ffi.rs, and
// kb_struct_sizes() lets the Rust test-suite verify the layouts match.

#ifndef KESTREL_KBRIDGE_H
#define KESTREL_KBRIDGE_H

#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>

#ifdef _WIN32
#  ifdef KBRIDGE_BUILDING
#    define KB_API __declspec(dllexport)
#  else
#    define KB_API __declspec(dllimport)
#  endif
#else
#  define KB_API __attribute__((visibility("default")))
#endif

#ifdef __cplusplus
extern "C" {
#endif

#define KB_ACCOUNT_ID_SIZE 8
#define KB_REGIST_KEY_SIZE 16
#define KB_MORNING_SIZE 16

/* Return codes are libchiaki's ChiakiErrorCode values (0 = success). */
KB_API const char *kb_error_string(int code);

/* Log levels match ChiakiLogLevel bit values. */
enum { KB_LOG_ERROR = 1, KB_LOG_WARNING = 2, KB_LOG_INFO = 4, KB_LOG_VERBOSE = 8, KB_LOG_DEBUG = 16 };
typedef void (*KbLogCb)(int level, const char *msg, void *user);

/* Must be called once per process before anything else. */
KB_API int kb_init(void);

/* Returns sizes of the bridge structs (for layout verification from Rust). */
typedef struct KbStructSizes {
	uint32_t host, regist_result, session_config, event, controller, stats;
} KbStructSizes;
KB_API void kb_struct_sizes(KbStructSizes *out);

/* ---------------------------------------------------------------- Discovery */

enum { KB_HOST_STATE_UNKNOWN = 0, KB_HOST_STATE_READY = 1, KB_HOST_STATE_STANDBY = 2 };

typedef struct KbHost {
	int32_t state;            /* KB_HOST_STATE_* */
	int32_t is_ps5;
	uint16_t request_port;
	char host_addr[64];
	char host_name[64];
	char host_id[32];         /* console MAC as hex, stable across IP changes */
	char host_type[16];
	char system_version[16];
	char running_app_titleid[32];
	char running_app_name[128];
} KbHost;

typedef void (*KbHostsCb)(const KbHost *hosts, size_t count, void *user);

typedef struct KbDiscovery KbDiscovery;

/*
 * Starts a background discovery service that pings every ping_ms.
 * manual_host == NULL → IPv4 broadcast (255.255.255.255) on the LAN.
 * manual_host != NULL → unicast probes to that IP/hostname only.
 * The callback runs on a libchiaki thread with the full current host list.
 */
KB_API KbDiscovery *kb_discovery_start(const char *manual_host, uint32_t ping_ms,
		KbHostsCb cb, void *cb_user, KbLogCb log_cb, void *log_user, int *err_out);
KB_API void kb_discovery_stop(KbDiscovery *d);

/* Sends the Remote Play wake packet. Needs the regist key from pairing. */
KB_API int kb_wakeup(const char *host, const char regist_key[KB_REGIST_KEY_SIZE], bool ps5,
		KbLogCb log_cb, void *log_user);

/* ------------------------------------------------------------- Registration */

enum { KB_REGIST_CANCELED = 0, KB_REGIST_FAILED = 1, KB_REGIST_SUCCESS = 2 };

typedef struct KbRegistResult {
	int32_t target;           /* ChiakiTarget */
	uint32_t rp_key_type;
	uint32_t console_pin;
	uint8_t server_mac[6];
	char server_nickname[32];
	char rp_regist_key[KB_REGIST_KEY_SIZE];  /* "regist key" — secret */
	uint8_t rp_key[KB_MORNING_SIZE];         /* "morning" — secret */
} KbRegistResult;

typedef void (*KbRegistCb)(int status, const KbRegistResult *result, void *user);

typedef struct KbRegist KbRegist;

/*
 * Pairs with a console that is showing Settings → System → Remote Play → Link Device.
 * pin: the 8-digit code on the TV. console_pin: the PS5 login passcode, or 0.
 * broadcast: true if host is a broadcast address.
 * The callback fires exactly once from a libchiaki thread.
 */
KB_API KbRegist *kb_regist_start(const char *host, bool broadcast, bool ps5,
		const uint8_t account_id[KB_ACCOUNT_ID_SIZE], uint32_t pin, uint32_t console_pin,
		KbRegistCb cb, void *cb_user, KbLogCb log_cb, void *log_user, int *err_out);
/* Cancels if still running, joins the worker and frees. Safe after the callback fired. */
KB_API void kb_regist_free(KbRegist *r);

/* ------------------------------------------------------------------ Session */

enum { KB_CODEC_H264 = 0, KB_CODEC_H265 = 1 };
enum { KB_RES_360P = 1, KB_RES_540P = 2, KB_RES_720P = 3, KB_RES_1080P = 4 };

typedef struct KbSessionConfig {
	const char *host;
	int32_t ps5;
	int32_t resolution;            /* KB_RES_* */
	int32_t fps;                   /* 30 or 60 */
	uint32_t bitrate_kbps;         /* 0 → preset default */
	int32_t codec;                 /* KB_CODEC_* */
	int32_t enable_dualsense;      /* haptics + adaptive trigger events */
	double packet_loss_max;        /* e.g. 0.05; congestion reports above this */
	char regist_key[KB_REGIST_KEY_SIZE];
	uint8_t morning[KB_MORNING_SIZE];
	uint8_t psn_account_id[KB_ACCOUNT_ID_SIZE];
} KbSessionConfig;

enum {
	KB_EVENT_CONNECTED = 0,
	KB_EVENT_LOGIN_PIN_REQUEST = 1,
	KB_EVENT_QUIT = 2,
	KB_EVENT_RUMBLE = 3,
	KB_EVENT_TRIGGER_EFFECTS = 4,
	KB_EVENT_LED_COLOR = 5,
	KB_EVENT_PLAYER_INDEX = 6,
	KB_EVENT_NICKNAME = 7,
	KB_EVENT_VIDEO_FEC_FAILURE = 8,
	KB_EVENT_OTHER = 99
};

typedef struct KbEvent {
	int32_t type;
	/* QUIT */
	int32_t quit_reason;           /* ChiakiQuitReason */
	int32_t quit_is_error;
	char quit_reason_str[256];
	/* LOGIN_PIN_REQUEST */
	int32_t pin_incorrect;
	/* RUMBLE */
	uint8_t rumble_left, rumble_right;
	/* TRIGGER_EFFECTS */
	uint8_t trigger_type_left, trigger_type_right;
	uint8_t trigger_left[10], trigger_right[10];
	/* LED_COLOR / PLAYER_INDEX */
	uint8_t led[3];
	uint8_t player_index;
	/* NICKNAME */
	char nickname[32];
} KbEvent;

/*
 * Video: one complete access unit (Annex-B NAL units) per call.
 * Return false if the frame could not be used; libchiaki then marks it corrupt
 * and requests a new keyframe.
 */
typedef bool (*KbVideoCb)(const uint8_t *buf, size_t size, int32_t frames_lost, bool frame_recovered, void *user);
typedef void (*KbEventCb)(const KbEvent *event, void *user);
typedef void (*KbAudioSettingsCb)(uint32_t channels, uint32_t rate, void *user);
/* Decoded interleaved signed 16-bit PCM. samples = frames per channel. */
typedef void (*KbAudioFrameCb)(const int16_t *pcm, size_t samples, void *user);
/* Raw DualSense haptics PCM stream (3 kHz, 2ch, s16) when enabled. */
typedef void (*KbHapticsCb)(const uint8_t *buf, size_t size, void *user);

typedef struct KbSessionCallbacks {
	void *user;
	KbEventCb event;
	KbVideoCb video;
	KbAudioSettingsCb audio_settings;
	KbAudioFrameCb audio_frame;
	KbHapticsCb haptics;     /* may be NULL */
	KbLogCb log;
} KbSessionCallbacks;

typedef struct KbControllerState {
	uint32_t buttons;              /* KB_BTN_* bitmask, same bits as libchiaki */
	uint8_t l2, r2;
	int16_t left_x, left_y, right_x, right_y;
	int8_t touch_id[2];            /* -1 = no touch */
	uint16_t touch_x[2], touch_y[2];
	float gyro_x, gyro_y, gyro_z;
	float accel_x, accel_y, accel_z;
	float orient_x, orient_y, orient_z, orient_w;
} KbControllerState;

typedef struct KbStats {
	uint64_t rtt_us;               /* measured by libchiaki during session setup */
	double measured_bitrate_mbps;  /* stream bitrate measured by libchiaki */
	double packet_loss;            /* 0..1, from libchiaki congestion control window */
	int32_t width, height, fps, bitrate_kbps; /* negotiated profile */
	int32_t codec;
} KbStats;

typedef struct KbSession KbSession;

KB_API KbSession *kb_session_create(const KbSessionConfig *cfg, const KbSessionCallbacks *cbs, int *err_out);
KB_API int kb_session_start(KbSession *s);
KB_API int kb_session_stop(KbSession *s);
/* Blocks until the session thread exits. Call after stop or after a QUIT event. */
KB_API int kb_session_join(KbSession *s);
KB_API void kb_session_free(KbSession *s);

KB_API int kb_session_set_controller(KbSession *s, const KbControllerState *state);
KB_API int kb_session_set_login_pin(KbSession *s, const char *pin);
KB_API int kb_session_request_idr(KbSession *s);
KB_API int kb_session_goto_bed(KbSession *s);
KB_API int kb_session_go_home(KbSession *s);
KB_API void kb_session_get_stats(KbSession *s, KbStats *out);

#ifdef __cplusplus
}
#endif

#endif
