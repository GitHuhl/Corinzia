// The test recorder plugin as a real, dlopen-able CLAP bundle
// (build/test-plugins/corinzia-test-recorder.clap).
#include "../Fixture.hpp"

extern "C" CLAP_EXPORT const clap_plugin_entry_t clap_entry = {
    CLAP_VERSION,
    fixture::kEntry.init,
    fixture::kEntry.deinit,
    fixture::kEntry.get_factory,
};
