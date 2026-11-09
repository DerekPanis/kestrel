// SPDX-License-Identifier: AGPL-3.0-only
// Implementation of the kbridge API on top of chiaki-ng's libchiaki.

#define KBRIDGE_BUILDING 1
#include "kbridge.h"

#include <chiaki/common.h>
#include <chiaki/log.h>
#include <chiaki/discovery.h>
#include <chiaki/discoveryservice.h>
#include <chiaki/regist.h>
#include <chiaki/session.h>
#include <chiaki/controller.h>
#include <chiaki/opusdecoder.h>

#include <stdlib.h>
#include <string.h>
#include <stdio.h>

#ifdef _WIN32
#include <winsock2.h>
#include <ws2tcpip.h>
#else
#include <netinet/in.h>
#include <arpa/inet.h>
#endif

/* ------------------------------------------------------------------ helpers */

typedef struct {
	KbLogCb cb;
	void *user;
} LogTarget;

static void log_trampoline(ChiakiLogLevel level, const char *msg, void *user)
{
	LogTarget *t = user;
	if(t && t->cb)
		t->cb((int)level, msg, t->user);
}

static void log_setup(ChiakiLog *log, LogTarget *target, KbLogCb cb, void *user)
{
	target->cb = cb;
	target->user = user;
	/* Debug-level output from libchiaki contains packet hexdumps; keep it out by default. */
	chiaki_log_init(log, CHIAKI_LOG_ALL & ~CHIAKI_LOG_DEBUG & ~CHIAKI_LOG_VERBOSE, log_trampoline, target);
}

static void copy_str(char *dst, size_t dst_size, const char *src)
{
	if(!dst_size)
		return;
	if(!src)
	{
		dst[0] = '\0';
		return;
	}
	size_t n = strlen(src);
	if(n >= dst_size)
		n = dst_size - 1;
	memcpy(dst, src, n);
	dst[n] = '\0';
}

static void set_err(int *err_out, int err)
{
	if(err_out)
		*err_out = err;
}

KB_API const char *kb_error_string(int code)
{
	return chiaki_error_string((ChiakiErrorCode)code);
}

KB_API int kb_init(void)
{
	return (int)chiaki_lib_init();
}

KB_API void kb_struct_sizes(KbStructSizes *out)
{
	out->host = (uint32_t)sizeof(KbHost);
	out->regist_result = (uint32_t)sizeof(KbRegistResult);
	out->session_config = (uint32_t)sizeof(KbSessionConfig);
	out->event = (uint32_t)sizeof(KbEvent);
	out->controller = (uint32_t)sizeof(KbControllerState);
	out->stats = (uint32_t)sizeof(KbStats);
}

/* ---------------------------------------------------------------- Discovery */

#define KB_DISCOVERY_HOSTS_MAX 16
#define KB_DISCOVERY_DROP_PINGS 3

struct KbDiscovery {
	ChiakiDiscoveryService service;
	ChiakiLog log;
	LogTarget log_target;
	KbHostsCb cb;
	void *cb_user;
};

static void discovery_hosts_cb(ChiakiDiscoveryHost *hosts, size_t count, void *user)
{
	KbDiscovery *d = user;
	if(!d->cb)
		return;
	KbHost out[KB_DISCOVERY_HOSTS_MAX];
	if(count > KB_DISCOVERY_HOSTS_MAX)
		count = KB_DISCOVERY_HOSTS_MAX;
	for(size_t i = 0; i < count; i++)
	{
		ChiakiDiscoveryHost *h = &hosts[i];
		KbHost *o = &out[i];
		memset(o, 0, sizeof(*o));
		o->state = (int32_t)h->state;
		o->is_ps5 = chiaki_discovery_host_is_ps5(h) ? 1 : 0;
		o->request_port = h->host_request_port;
		copy_str(o->host_addr, sizeof(o->host_addr), h->host_addr);
		copy_str(o->host_name, sizeof(o->host_name), h->host_name);
		copy_str(o->host_id, sizeof(o->host_id), h->host_id);
		copy_str(o->host_type, sizeof(o->host_type), h->host_type);
		copy_str(o->system_version, sizeof(o->system_version), h->system_version);
		copy_str(o->running_app_titleid, sizeof(o->running_app_titleid), h->running_app_titleid);
		copy_str(o->running_app_name, sizeof(o->running_app_name), h->running_app_name);
	}
	d->cb(out, count, d->cb_user);
}

KB_API KbDiscovery *kb_discovery_start(const char *manual_host, uint32_t ping_ms,
		KbHostsCb cb, void *cb_user, KbLogCb log_cb, void *log_user, int *err_out)
{
	KbDiscovery *d = calloc(1, sizeof(KbDiscovery));
	if(!d)
	{
		set_err(err_out, CHIAKI_ERR_MEMORY);
		return NULL;
	}
	log_setup(&d->log, &d->log_target, log_cb, log_user);
	d->cb = cb;
	d->cb_user = cb_user;

	ChiakiDiscoveryServiceOptions options;
	memset(&options, 0, sizeof(options));
	options.ping_ms = ping_ms ? ping_ms : 500;
	options.ping_initial_ms = options.ping_ms;
	options.host_drop_pings = KB_DISCOVERY_DROP_PINGS;
	options.cb = discovery_hosts_cb;
	options.cb_user = d;

	/* Options are copied by chiaki_discovery_service_init (send_host is strdup'd). */
	struct sockaddr_storage addr;
	memset(&addr, 0, sizeof(addr));
	if(manual_host && manual_host[0])
	{
		options.hosts_max = 1;
		options.send_host = (char *)manual_host;
		options.send_addr = &addr;
		if(strchr(manual_host, ':'))
		{
			addr.ss_family = AF_INET6;
			options.send_addr_size = sizeof(struct sockaddr_in6);
		}
		else
		{
			addr.ss_family = AF_INET;
			options.send_addr_size = sizeof(struct sockaddr_in);
		}
	}
	else
	{
		options.hosts_max = KB_DISCOVERY_HOSTS_MAX;
		struct sockaddr_in in_addr;
		memset(&in_addr, 0, sizeof(in_addr));
		in_addr.sin_family = AF_INET;
		in_addr.sin_addr.s_addr = 0xffffffff; /* 255.255.255.255 */
		memcpy(&addr, &in_addr, sizeof(in_addr));
		options.send_addr = &addr;
		options.send_addr_size = sizeof(in_addr);
	}

	ChiakiErrorCode err = chiaki_discovery_service_init(&d->service, &options, &d->log);
	if(err != CHIAKI_ERR_SUCCESS)
	{
		free(d);
		set_err(err_out, err);
		return NULL;
	}
	set_err(err_out, CHIAKI_ERR_SUCCESS);
	return d;
}

KB_API void kb_discovery_stop(KbDiscovery *d)
{
	if(!d)
		return;
	chiaki_discovery_service_fini(&d->service);
	free(d);
}

KB_API int kb_wakeup(const char *host, const char regist_key[KB_REGIST_KEY_SIZE], bool ps5,
		KbLogCb log_cb, void *log_user)
{
	ChiakiLog log;
	LogTarget target;
	log_setup(&log, &target, log_cb, log_user);

	/* The wake credential is the regist key (an ASCII hex string, \0-padded) parsed as hex. */
	char key[KB_REGIST_KEY_SIZE + 1];
	memcpy(key, regist_key, KB_REGIST_KEY_SIZE);
	key[KB_REGIST_KEY_SIZE] = '\0';
	size_t len = strlen(key);
	if(len == 0 || len > 8)
		return CHIAKI_ERR_INVALID_DATA;
	char *end = NULL;
	uint64_t credential = (uint64_t)strtoull(key, &end, 16);
	if(!end || *end != '\0')
		return CHIAKI_ERR_INVALID_DATA;
	return (int)chiaki_discovery_wakeup(&log, NULL, host, credential, ps5);
}

/* ------------------------------------------------------------- Registration */

struct KbRegist {
	ChiakiRegist regist;
	ChiakiLog log;
	LogTarget log_target;
	KbRegistCb cb;
	void *cb_user;
	char *host;
};

static void regist_cb(ChiakiRegistEvent *event, void *user)
{
	KbRegist *r = user;
	if(!r->cb)
		return;
	switch(event->type)
	{
		case CHIAKI_REGIST_EVENT_TYPE_FINISHED_SUCCESS:
		{
			KbRegistResult res;
			memset(&res, 0, sizeof(res));
			ChiakiRegisteredHost *h = event->registered_host;
			res.target = (int32_t)h->target;
			res.rp_key_type = h->rp_key_type;
			res.console_pin = h->console_pin;
			memcpy(res.server_mac, h->server_mac, sizeof(res.server_mac));
			copy_str(res.server_nickname, sizeof(res.server_nickname), h->server_nickname);
			memcpy(res.rp_regist_key, h->rp_regist_key, sizeof(res.rp_regist_key));
			memcpy(res.rp_key, h->rp_key, sizeof(res.rp_key));
			r->cb(KB_REGIST_SUCCESS, &res, r->cb_user);
			break;
		}
		case CHIAKI_REGIST_EVENT_TYPE_FINISHED_FAILED:
			r->cb(KB_REGIST_FAILED, NULL, r->cb_user);
			break;
		default:
			r->cb(KB_REGIST_CANCELED, NULL, r->cb_user);
			break;
	}
}

KB_API KbRegist *kb_regist_start(const char *host, bool broadcast, bool ps5,
		const uint8_t account_id[KB_ACCOUNT_ID_SIZE], uint32_t pin, uint32_t console_pin,
		KbRegistCb cb, void *cb_user, KbLogCb log_cb, void *log_user, int *err_out)
{
	KbRegist *r = calloc(1, sizeof(KbRegist));
	if(!r)
	{
		set_err(err_out, CHIAKI_ERR_MEMORY);
		return NULL;
	}
	r->host = strdup(host);
	if(!r->host)
	{
		free(r);
		set_err(err_out, CHIAKI_ERR_MEMORY);
		return NULL;
	}
	log_setup(&r->log, &r->log_target, log_cb, log_user);
	r->cb = cb;
	r->cb_user = cb_user;

	ChiakiRegistInfo info;
	memset(&info, 0, sizeof(info));
	info.target = ps5 ? CHIAKI_TARGET_PS5_1 : CHIAKI_TARGET_PS4_10;
	info.host = r->host;
	info.broadcast = broadcast;
	info.psn_online_id = NULL; /* account-id based registration (PS4 >= 7.0, all PS5) */
	memcpy(info.psn_account_id, account_id, KB_ACCOUNT_ID_SIZE);
	info.pin = pin;
	info.console_pin = console_pin;
	info.holepunch_info = NULL;
	info.rudp = NULL;

	ChiakiErrorCode err = chiaki_regist_start(&r->regist, &r->log, &info, regist_cb, r);
	if(err != CHIAKI_ERR_SUCCESS)
	{
		free(r->host);
		free(r);
		set_err(err_out, err);
		return NULL;
	}
	set_err(err_out, CHIAKI_ERR_SUCCESS);
	return r;
}

KB_API void kb_regist_free(KbRegist *r)
{
	if(!r)
		return;
	chiaki_regist_stop(&r->regist);
	chiaki_regist_fini(&r->regist); /* joins the worker thread */
	free(r->host);
	free(r);
}

/* ------------------------------------------------------------------ Session */

struct KbSession {
	ChiakiSession session;
	ChiakiLog log;
	LogTarget log_target;
	ChiakiOpusDecoder opus;
	KbSessionCallbacks cbs;
	char *host;
	bool started;
};

static void session_event_cb(ChiakiEvent *event, void *user)
{
	KbSession *s = user;
	if(!s->cbs.event)
		return;
	KbEvent e;
	memset(&e, 0, sizeof(e));
	switch(event->type)
	{
		case CHIAKI_EVENT_CONNECTED:
			e.type = KB_EVENT_CONNECTED;
			break;
		case CHIAKI_EVENT_LOGIN_PIN_REQUEST:
			e.type = KB_EVENT_LOGIN_PIN_REQUEST;
			e.pin_incorrect = event->login_pin_request.pin_incorrect ? 1 : 0;
			break;
		case CHIAKI_EVENT_QUIT:
			e.type = KB_EVENT_QUIT;
			e.quit_reason = (int32_t)event->quit.reason;
			e.quit_is_error = chiaki_quit_reason_is_error(event->quit.reason) ? 1 : 0;
			copy_str(e.quit_reason_str, sizeof(e.quit_reason_str),
					event->quit.reason_str ? event->quit.reason_str : chiaki_quit_reason_string(event->quit.reason));
			break;
		case CHIAKI_EVENT_RUMBLE:
			e.type = KB_EVENT_RUMBLE;
			e.rumble_left = event->rumble.left;
			e.rumble_right = event->rumble.right;
			break;
		case CHIAKI_EVENT_TRIGGER_EFFECTS:
			e.type = KB_EVENT_TRIGGER_EFFECTS;
			e.trigger_type_left = event->trigger_effects.type_left;
			e.trigger_type_right = event->trigger_effects.type_right;
			memcpy(e.trigger_left, event->trigger_effects.left, sizeof(e.trigger_left));
			memcpy(e.trigger_right, event->trigger_effects.right, sizeof(e.trigger_right));
			break;
		case CHIAKI_EVENT_LED_COLOR:
			e.type = KB_EVENT_LED_COLOR;
			memcpy(e.led, event->led_state, sizeof(e.led));
			break;
		case CHIAKI_EVENT_PLAYER_INDEX:
			e.type = KB_EVENT_PLAYER_INDEX;
			e.player_index = event->player_index;
			break;
		case CHIAKI_EVENT_NICKNAME_RECEIVED:
			e.type = KB_EVENT_NICKNAME;
			copy_str(e.nickname, sizeof(e.nickname), event->server_nickname);
			break;
		case CHIAKI_EVENT_VIDEO_FEC_FAILURE:
			e.type = KB_EVENT_VIDEO_FEC_FAILURE;
			break;
		default:
			e.type = KB_EVENT_OTHER;
			break;
	}
	s->cbs.event(&e, s->cbs.user);
}

static bool session_video_cb(uint8_t *buf, size_t buf_size, int32_t frames_lost, bool frame_recovered, void *user)
{
	KbSession *s = user;
	if(!s->cbs.video)
		return true;
	return s->cbs.video(buf, buf_size, frames_lost, frame_recovered, s->cbs.user);
}

static void opus_settings_cb(uint32_t channels, uint32_t rate, void *user)
{
	KbSession *s = user;
	if(s->cbs.audio_settings)
		s->cbs.audio_settings(channels, rate, s->cbs.user);
}

static void opus_frame_cb(int16_t *buf, size_t samples_count, void *user)
{
	KbSession *s = user;
	if(s->cbs.audio_frame)
		s->cbs.audio_frame(buf, samples_count, s->cbs.user);
}

static void haptics_frame_cb(uint8_t *buf, size_t buf_size, void *user)
{
	KbSession *s = user;
	if(s->cbs.haptics)
		s->cbs.haptics(buf, buf_size, s->cbs.user);
}

KB_API KbSession *kb_session_create(const KbSessionConfig *cfg, const KbSessionCallbacks *cbs, int *err_out)
{
	if(!cfg || !cfg->host || !cbs)
	{
		set_err(err_out, CHIAKI_ERR_INVALID_DATA);
		return NULL;
	}
	if(cfg->fps != 30 && cfg->fps != 60)
	{
		set_err(err_out, CHIAKI_ERR_INVALID_DATA);
		return NULL;
	}
	if(cfg->resolution < KB_RES_360P || cfg->resolution > KB_RES_1080P)
	{
		set_err(err_out, CHIAKI_ERR_INVALID_DATA);
		return NULL;
	}

	KbSession *s = calloc(1, sizeof(KbSession));
	if(!s)
	{
		set_err(err_out, CHIAKI_ERR_MEMORY);
		return NULL;
	}
	s->cbs = *cbs;
	s->host = strdup(cfg->host);
	if(!s->host)
	{
		free(s);
		set_err(err_out, CHIAKI_ERR_MEMORY);
		return NULL;
	}
	log_setup(&s->log, &s->log_target, cbs->log, cbs->user);

	ChiakiConnectInfo info;
	memset(&info, 0, sizeof(info));
	info.ps5 = cfg->ps5 != 0;
	info.host = s->host;
	memcpy(info.regist_key, cfg->regist_key, sizeof(info.regist_key));
	memcpy(info.morning, cfg->morning, sizeof(info.morning));
	memcpy(info.psn_account_id, cfg->psn_account_id, sizeof(info.psn_account_id));
	chiaki_connect_video_profile_preset(&info.video_profile,
			(ChiakiVideoResolutionPreset)cfg->resolution,
			cfg->fps == 30 ? CHIAKI_VIDEO_FPS_PRESET_30 : CHIAKI_VIDEO_FPS_PRESET_60);
	if(cfg->bitrate_kbps)
		info.video_profile.bitrate = cfg->bitrate_kbps;
	/* H.265 is PS5-only. */
	info.video_profile.codec = (info.ps5 && cfg->codec == KB_CODEC_H265) ? CHIAKI_CODEC_H265 : CHIAKI_CODEC_H264;
	info.video_profile_auto_downgrade = true;
	info.enable_keyboard = false;
	info.enable_dualsense = cfg->enable_dualsense != 0;
	info.audio_video_disabled = 0;
	info.auto_regist = false;
	info.holepunch_session = NULL;
	info.rudp_sock = NULL;
	info.packet_loss_max = cfg->packet_loss_max > 0 ? cfg->packet_loss_max : 0.05;
	info.enable_idr_on_fec_failure = true;

	ChiakiErrorCode err = chiaki_session_init(&s->session, &info, &s->log);
	if(err != CHIAKI_ERR_SUCCESS)
	{
		free(s->host);
		free(s);
		set_err(err_out, err);
		return NULL;
	}

	chiaki_opus_decoder_init(&s->opus, &s->log);
	chiaki_opus_decoder_set_cb(&s->opus, opus_settings_cb, opus_frame_cb, s);
	ChiakiAudioSink audio_sink;
	memset(&audio_sink, 0, sizeof(audio_sink));
	chiaki_opus_decoder_get_sink(&s->opus, &audio_sink);
	chiaki_session_set_audio_sink(&s->session, &audio_sink);

	if(cfg->enable_dualsense && cbs->haptics)
	{
		ChiakiAudioSink haptics_sink;
		memset(&haptics_sink, 0, sizeof(haptics_sink));
		haptics_sink.user = s;
		haptics_sink.frame_cb = haptics_frame_cb;
		chiaki_session_set_haptics_sink(&s->session, &haptics_sink);
	}

	chiaki_session_set_video_sample_cb(&s->session, session_video_cb, s);
	chiaki_session_set_event_cb(&s->session, session_event_cb, s);

	set_err(err_out, CHIAKI_ERR_SUCCESS);
	return s;
}

KB_API int kb_session_start(KbSession *s)
{
	ChiakiErrorCode err = chiaki_session_start(&s->session);
	if(err == CHIAKI_ERR_SUCCESS)
		s->started = true;
	return (int)err;
}

KB_API int kb_session_stop(KbSession *s)
{
	return (int)chiaki_session_stop(&s->session);
}

KB_API int kb_session_join(KbSession *s)
{
	if(!s->started)
		return CHIAKI_ERR_SUCCESS;
	ChiakiErrorCode err = chiaki_session_join(&s->session);
	s->started = false;
	return (int)err;
}

KB_API void kb_session_free(KbSession *s)
{
	if(!s)
		return;
	if(s->started)
	{
		chiaki_session_stop(&s->session);
		chiaki_session_join(&s->session);
	}
	chiaki_session_fini(&s->session);
	chiaki_opus_decoder_fini(&s->opus);
	free(s->host);
	free(s);
}

KB_API int kb_session_set_controller(KbSession *s, const KbControllerState *in)
{
	ChiakiControllerState st;
	chiaki_controller_state_set_idle(&st);
	st.buttons = in->buttons;
	st.l2_state = in->l2;
	st.r2_state = in->r2;
	st.left_x = in->left_x;
	st.left_y = in->left_y;
	st.right_x = in->right_x;
	st.right_y = in->right_y;
	for(int i = 0; i < CHIAKI_CONTROLLER_TOUCHES_MAX && i < 2; i++)
	{
		st.touches[i].id = in->touch_id[i];
		st.touches[i].x = in->touch_x[i];
		st.touches[i].y = in->touch_y[i];
	}
	st.gyro_x = in->gyro_x; st.gyro_y = in->gyro_y; st.gyro_z = in->gyro_z;
	st.accel_x = in->accel_x; st.accel_y = in->accel_y; st.accel_z = in->accel_z;
	/* An all-zero quaternion is invalid; keep libchiaki's idle orientation unless one is supplied. */
	if(in->orient_w != 0.0f || in->orient_x != 0.0f || in->orient_y != 0.0f || in->orient_z != 0.0f)
	{
		st.orient_x = in->orient_x; st.orient_y = in->orient_y;
		st.orient_z = in->orient_z; st.orient_w = in->orient_w;
	}
	return (int)chiaki_session_set_controller_state(&s->session, &st);
}

KB_API int kb_session_set_login_pin(KbSession *s, const char *pin)
{
	if(!pin)
		return CHIAKI_ERR_INVALID_DATA;
	return (int)chiaki_session_set_login_pin(&s->session, (const uint8_t *)pin, strlen(pin));
}

KB_API int kb_session_request_idr(KbSession *s)
{
	return (int)chiaki_session_request_idr(&s->session);
}

KB_API int kb_session_goto_bed(KbSession *s)
{
	return (int)chiaki_session_goto_bed(&s->session);
}

KB_API int kb_session_go_home(KbSession *s)
{
	return (int)chiaki_session_go_home(&s->session);
}

KB_API void kb_session_get_stats(KbSession *s, KbStats *out)
{
	/* These fields are written by libchiaki's own threads; reading them
	 * unlocked can at worst yield a value from the previous update. */
	memset(out, 0, sizeof(*out));
	out->rtt_us = s->session.rtt_us;
	out->measured_bitrate_mbps = s->session.stream_connection.measured_bitrate;
	out->packet_loss = s->session.stream_connection.congestion_control.packet_loss;
	out->width = (int32_t)s->session.connect_info.video_profile.width;
	out->height = (int32_t)s->session.connect_info.video_profile.height;
	out->fps = (int32_t)s->session.connect_info.video_profile.max_fps;
	out->bitrate_kbps = (int32_t)s->session.connect_info.video_profile.bitrate;
	out->codec = (int32_t)s->session.connect_info.video_profile.codec;
}
