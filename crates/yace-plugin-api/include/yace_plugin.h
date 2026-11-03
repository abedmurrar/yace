/*
 * YACE native plugin ABI, version 1 (C header).
 *
 * Mirrors crates/yace-plugin-api/src/lib.rs exactly; a test compiles this
 * header and checks every struct size and field offset against the Rust
 * definitions. See that file and docs/plugins.md for the rules (sizes,
 * ownership, threading, cancellation).
 *
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */
#ifndef YACE_PLUGIN_H
#define YACE_PLUGIN_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define YACE_ABI_VERSION 1u
#define YACE_INIT_SYMBOL "yace_plugin_init"

typedef int32_t YaceStatus;
#define YACE_OK 0
#define YACE_E_FAILED 1
#define YACE_E_ABI 2
#define YACE_E_INVALID 3
#define YACE_E_NOT_FOUND 4
#define YACE_E_STALE 5
#define YACE_E_DENIED 6
#define YACE_E_UNSUPPORTED 7
#define YACE_E_CANCELLED 8
#define YACE_E_PANIC 9

#define YACE_LEVEL_DEBUG 0u
#define YACE_LEVEL_INFO 1u
#define YACE_LEVEL_WARNING 2u
#define YACE_LEVEL_ERROR 3u

typedef uint64_t YaceHandle;

/* Borrowed UTF-8, not NUL-terminated; valid for the call only. */
typedef struct YaceStr {
    const uint8_t *ptr;
    size_t len;
} YaceStr;

/* Filled by the host; give it back with host->free. */
typedef struct YaceBuf {
    uint8_t *ptr;
    size_t len;
    size_t cap;
} YaceBuf;

typedef struct YaceHostCtx YaceHostCtx;

typedef void (*YaceJobFn)(void *data, YaceHandle job);

typedef struct YaceHost {
    uint32_t size;
    uint32_t abi_version;
    YaceHostCtx *ctx;
    void (*log)(YaceHostCtx *ctx, uint32_t level, YaceStr message);
    void (*notify)(YaceHostCtx *ctx, uint32_t level, YaceStr message);
    YaceStr (*last_error)(YaceHostCtx *ctx);
    void (*free)(YaceHostCtx *ctx, YaceBuf buf);
    YaceStatus (*get_setting)(YaceHostCtx *ctx, YaceStr key, YaceBuf *out);
    YaceStatus (*workspace_roots)(YaceHostCtx *ctx, YaceBuf *out);
    YaceStatus (*active_document)(YaceHostCtx *ctx, YaceBuf *out);
    YaceStatus (*document_text)(YaceHostCtx *ctx, YaceHandle doc, uint64_t start, uint64_t end, YaceBuf *out);
    YaceStatus (*apply_edits)(YaceHostCtx *ctx, YaceHandle doc, uint64_t version, YaceStr edits);
    YaceStatus (*open_file)(YaceHostCtx *ctx, YaceStr path, uint32_t line, uint32_t column);
    YaceStatus (*execute_command)(YaceHostCtx *ctx, YaceStr id, YaceStr args);
    YaceStatus (*set_status_item)(YaceHostCtx *ctx, YaceStr id, YaceStr item);
    YaceStatus (*set_view_items)(YaceHostCtx *ctx, YaceStr view, YaceStr items);
    YaceStatus (*set_decorations)(YaceHostCtx *ctx, YaceHandle root, YaceStr items);
    YaceStatus (*quick_open_results)(YaceHostCtx *ctx, uint64_t request, YaceStr items);
    YaceStatus (*create_terminal)(YaceHostCtx *ctx, YaceStr spec, YaceHandle *out);
    YaceStatus (*spawn_job)(YaceHostCtx *ctx, YaceJobFn job, void *data, YaceHandle *out);
    bool (*job_cancelled)(YaceHostCtx *ctx, YaceHandle job);
    YaceStatus (*post_message)(YaceHostCtx *ctx, YaceStr data);
} YaceHost;

/* Every callback may be NULL. */
typedef struct YacePlugin {
    uint32_t size;
    uint32_t abi_version;
    void *instance;
    YaceStatus (*activate)(void *instance);
    void (*deactivate)(void *instance);
    YaceStatus (*on_command)(void *instance, YaceStr id, YaceStr args);
    void (*on_event)(void *instance, YaceStr event);
    void (*on_message)(void *instance, YaceStr data);
    YaceStatus (*on_view_action)(void *instance, YaceStr view, YaceStr item, YaceStr action);
    YaceStatus (*on_quick_open)(void *instance, YaceStr provider, YaceStr query, uint64_t request);
    void (*destroy)(void *instance);
} YacePlugin;

/* Export this. Write at most out->size bytes; set out->size to what you wrote. */
YaceStatus yace_plugin_init(uint32_t abi, const YaceHost *host, YacePlugin *out);

#ifdef __cplusplus
}
#endif

#endif /* YACE_PLUGIN_H */
