/* C interface of the Rust export crate (rust/corinzia-export). */
#ifndef CORINZIA_EXPORT_H
#define CORINZIA_EXPORT_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* `json`: an export document (see rust/corinzia-export/src/doc.rs).
 * Return 0 on success; otherwise non-zero with a NUL-terminated message in
 * `err` (truncated to `err_len`). */
int cz_export_midi(const unsigned char* json, size_t len, const char* path, char* err, size_t err_len);
int cz_export_musicxml(const unsigned char* json, size_t len, const char* path, char* err, size_t err_len);

#ifdef __cplusplus
}
#endif

#endif
