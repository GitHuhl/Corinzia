/* A stand-in libpipewire-0.3.so.0 for the AppImage, used only on systems
 * without PipeWire: every call fails cleanly, so Corinzia starts and
 * reports "no audio output" instead of failing to load. */
#include <stddef.h>

static int dummy_properties;

void pw_init(int *argc, char **argv[]) { (void)argc; (void)argv; }
void *pw_main_loop_new(const void *props) { (void)props; return NULL; }
void *pw_main_loop_get_loop(void *loop) { (void)loop; return NULL; }
int pw_main_loop_run(void *loop) { (void)loop; return -1; }
int pw_main_loop_quit(void *loop) { (void)loop; return -1; }
void pw_main_loop_destroy(void *loop) { (void)loop; }
void *pw_context_new(void *loop, void *props, size_t size) { (void)loop; (void)props; (void)size; return NULL; }
void *pw_context_connect(void *ctx, void *props, size_t size) { (void)ctx; (void)props; (void)size; return NULL; }
void pw_context_destroy(void *ctx) { (void)ctx; }
void *pw_properties_new(const char *key, ...) { (void)key; return &dummy_properties; }
int pw_properties_set(void *props, const char *key, const char *value) { (void)props; (void)key; (void)value; return 0; }
void pw_properties_free(void *props) { (void)props; }
void *pw_stream_new(void *core, const char *name, void *props) { (void)core; (void)name; (void)props; return NULL; }
void pw_stream_add_listener(void *s, void *l, const void *e, void *d) { (void)s; (void)l; (void)e; (void)d; }
int pw_stream_connect(void *s, int dir, unsigned id, int flags, const void **params, unsigned n) { (void)s; (void)dir; (void)id; (void)flags; (void)params; (void)n; return -1; }
void *pw_stream_dequeue_buffer(void *s) { (void)s; return NULL; }
int pw_stream_queue_buffer(void *s, void *b) { (void)s; (void)b; return -1; }
void pw_stream_destroy(void *s) { (void)s; }
