/*
 * Browser HEIF/HEIC/AVIF decode glue over the libheif C API.
 *
 * One decode result is retained in module state until hwc_release(). Status
 * codes are stable: 0 ok, 1 unsupported or missing decoder, 2 malformed input,
 * 3 resource limit, 4 out of memory. Pixels are tightly packed RGBA, 8 bits per
 * channel, with irot/imir transforms applied by libheif.
 */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <libheif/heif.h>

enum hwc_status {
    HWC_OK = 0,
    HWC_UNSUPPORTED = 1,
    HWC_INVALID = 2,
    HWC_LIMIT = 3,
    HWC_NO_MEMORY = 4
};

typedef struct {
    int32_t status;
    uint32_t width;
    uint32_t height;
    uint8_t *pixels;
    char message[192];
} hwc_result;

static hwc_result g_result;

static void set_message(const char *text)
{
    snprintf(g_result.message, sizeof g_result.message, "%s", text != NULL ? text : "");
}

static void release_pixels(void)
{
    free(g_result.pixels);
    g_result.pixels = NULL;
    g_result.width = 0;
    g_result.height = 0;
}

static int mul_u64(uint64_t a, uint64_t b, uint64_t *out)
{
    if (a != 0 && b > UINT64_MAX / a) {
        return 0;
    }
    *out = a * b;
    return 1;
}

static int32_t fail(int32_t status, const char *message)
{
    set_message(message);
    return status;
}

static int32_t from_heif(struct heif_error err)
{
    if (err.subcode == heif_suberror_Security_limit_exceeded) {
        set_message(err.message != NULL ? err.message : "libheif security limit exceeded");
        return HWC_LIMIT;
    }
    switch (err.code) {
    case heif_error_Memory_allocation_error:
        set_message(err.message != NULL ? err.message : "out of memory");
        return HWC_NO_MEMORY;
    case heif_error_Unsupported_feature:
        set_message(err.message != NULL ? err.message : "unsupported feature or missing decoder");
        return HWC_UNSUPPORTED;
    default:
        set_message(err.message != NULL ? err.message : "malformed input");
        return HWC_INVALID;
    }
}

static int within_limit(uint64_t width, uint64_t height, uint32_t max_pixels)
{
    uint64_t pixels = 0;
    return mul_u64(width, height, &pixels) && pixels <= (uint64_t)max_pixels;
}

/*
 * libheif's default ceiling (about 1.07e9 pixels) lets a small ispe hide a large
 * coded image until after the decode allocates. The ceiling is set to twice
 * max_pixels because libheif compares it with its padded internal size, so
 * max_pixels itself would reject an image that exactly meets the budget.
 */
static int32_t apply_pixel_ceiling(struct heif_context *ctx, uint32_t max_pixels)
{
    heif_security_limits *limits = heif_context_get_security_limits(ctx);
    uint64_t ceiling = 0;
    struct heif_error err;

    if (limits == NULL) {
        return fail(HWC_LIMIT, "libheif security limits are unavailable");
    }
    if (!mul_u64((uint64_t)max_pixels, 2, &ceiling)) {
        return fail(HWC_LIMIT, "pixel limit overflows the libheif ceiling");
    }
    limits->max_image_size_pixels = ceiling;
    err = heif_context_set_security_limits(ctx, limits);
    return err.code == heif_error_Ok ? HWC_OK : from_heif(err);
}

int32_t hwc_decode(const uint8_t *data, uint32_t size, uint32_t max_pixels)
{
    struct heif_context *ctx = NULL;
    struct heif_image_handle *handle = NULL;
    struct heif_image *image = NULL;
    struct heif_error err;
    int32_t status = HWC_OK;
    int initialized = 0;
    int width;
    int height;
    size_t stride = 0;
    const uint8_t *src;
    uint64_t row_bytes = 0;
    uint64_t total_bytes = 0;
    uint64_t plane_bytes = 0;
    uint64_t source_span = 0;
    uint8_t *dst;
    int y;

    release_pixels();
    set_message("");

    if (data == NULL || size == 0) {
        status = fail(HWC_INVALID, "input is empty");
        goto done;
    }
    if (max_pixels == 0) {
        status = fail(HWC_LIMIT, "pixel limit must be positive");
        goto done;
    }

    err = heif_init(NULL);
    if (err.code != heif_error_Ok) {
        status = from_heif(err);
        goto done;
    }
    initialized = 1;

    ctx = heif_context_alloc();
    if (ctx == NULL) {
        status = fail(HWC_NO_MEMORY, "out of memory");
        goto done;
    }

    status = apply_pixel_ceiling(ctx, max_pixels);
    if (status != HWC_OK) {
        goto done;
    }

    err = heif_context_read_from_memory_without_copy(ctx, data, size, NULL);
    if (err.code != heif_error_Ok) {
        status = from_heif(err);
        goto done;
    }

    err = heif_context_get_primary_image_handle(ctx, &handle);
    if (err.code != heif_error_Ok) {
        status = from_heif(err);
        goto done;
    }

    width = heif_image_handle_get_width(handle);
    height = heif_image_handle_get_height(handle);
    if (width <= 0 || height <= 0) {
        status = fail(HWC_INVALID, "primary image has no dimensions");
        goto done;
    }
    if (!within_limit((uint64_t)width, (uint64_t)height, max_pixels)) {
        status = fail(HWC_LIMIT, "primary image exceeds the pixel limit");
        goto done;
    }

    err = heif_decode_image(handle, &image, heif_colorspace_RGB,
                            heif_chroma_interleaved_RGBA, NULL);
    if (err.code != heif_error_Ok) {
        status = from_heif(err);
        goto done;
    }

    width = heif_image_get_width(image, heif_channel_interleaved);
    height = heif_image_get_height(image, heif_channel_interleaved);
    if (width <= 0 || height <= 0) {
        status = fail(HWC_INVALID, "decoded image has no dimensions");
        goto done;
    }
    if (!within_limit((uint64_t)width, (uint64_t)height, max_pixels)) {
        status = fail(HWC_LIMIT, "decoded image exceeds the pixel limit");
        goto done;
    }

    src = heif_image_get_plane_readonly2(image, heif_channel_interleaved, &stride);
    if (src == NULL) {
        status = fail(HWC_INVALID, "decoded image has no interleaved plane");
        goto done;
    }
    if (!mul_u64((uint64_t)width, 4, &row_bytes) ||
        !mul_u64(row_bytes, (uint64_t)height, &total_bytes) ||
        total_bytes > SIZE_MAX) {
        status = fail(HWC_LIMIT, "output size overflows the address space");
        goto done;
    }
    if ((uint64_t)stride < row_bytes ||
        !mul_u64((uint64_t)stride, (uint64_t)height, &plane_bytes) ||
        !mul_u64((uint64_t)stride, (uint64_t)(height - 1), &source_span) ||
        source_span > plane_bytes - row_bytes) {
        status = fail(HWC_INVALID, "decoded plane is smaller than its stride and height");
        goto done;
    }

    dst = malloc((size_t)total_bytes);
    if (dst == NULL) {
        status = fail(HWC_NO_MEMORY, "out of memory allocating RGBA output");
        goto done;
    }
    for (y = 0; y < height; y++) {
        memcpy(dst + (size_t)y * (size_t)row_bytes,
               src + (size_t)y * stride,
               (size_t)row_bytes);
    }

    g_result.pixels = dst;
    g_result.width = (uint32_t)width;
    g_result.height = (uint32_t)height;
    set_message("ok");

done:
    if (image != NULL) {
        heif_image_release(image);
    }
    if (handle != NULL) {
        heif_image_handle_release(handle);
    }
    if (ctx != NULL) {
        heif_context_free(ctx);
    }
    if (initialized) {
        heif_deinit();
    }
    if (status != HWC_OK) {
        release_pixels();
    }
    g_result.status = status;
    return status;
}

uint32_t hwc_width(void)
{
    return g_result.width;
}

uint32_t hwc_height(void)
{
    return g_result.height;
}

const uint8_t *hwc_pixels(void)
{
    return g_result.pixels;
}

const char *hwc_message(void)
{
    return g_result.message;
}

void hwc_release(void)
{
    release_pixels();
    set_message("");
    g_result.status = HWC_OK;
}
