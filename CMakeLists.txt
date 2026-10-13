cmake_minimum_required(VERSION 3.24)
project(Corinzia VERSION 0.1.0 LANGUAGES CXX)

# All builds are expected to run inside `nix develop` (see flake.nix), which
# provides CLAP, SQLite, PipeWire and Qt 6 through pkg-config / CMake configs.

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

option(CORINZIA_BUILD_TESTS "Build unit tests" ON)
option(CORINZIA_BUILD_UI "Build the Qt Quick front-end" ON)

find_package(PkgConfig REQUIRED)
find_package(Threads REQUIRED)
pkg_check_modules(CLAP REQUIRED IMPORTED_TARGET clap>=1.2)
pkg_check_modules(SQLITE REQUIRED IMPORTED_TARGET sqlite3>=3.45)

# Embed schema/project.sql so the binary can create/migrate .project files.
file(READ ${CMAKE_SOURCE_DIR}/schema/project.sql CORINZIA_SCHEMA_SQL_TEXT)
configure_file(src/storage/SchemaSql.cpp.in ${CMAKE_BINARY_DIR}/generated/SchemaSql.cpp @ONLY)
set_property(DIRECTORY APPEND PROPERTY CMAKE_CONFIGURE_DEPENDS schema/project.sql)

# ---------------------------------------------------------------- core ----
# Qt-free: domain model, RT engine, CLAP hosting, snapshot + notation compilers.
add_library(corinzia_core
    src/core/Types.cpp
    src/model/Model.cpp
    src/model/Commands.cpp
    src/audio/ClapPluginHost.cpp
    src/audio/AudioEngine.cpp
    src/compile/SnapshotCompiler.cpp
    src/engrave/LilyPondEmitter.cpp
    src/storage/ProjectStore.cpp
    ${CMAKE_BINARY_DIR}/generated/SchemaSql.cpp
)
target_include_directories(corinzia_core PUBLIC include)
target_link_libraries(corinzia_core PUBLIC PkgConfig::CLAP PkgConfig::SQLITE ${CMAKE_DL_LIBS} Threads::Threads)
target_compile_options(corinzia_core PRIVATE -Wall -Wextra -Wpedantic)

# ------------------------------------------------------------------ ui ----
if(CORINZIA_BUILD_UI)
    find_package(Qt6 6.5 REQUIRED COMPONENTS Core Gui Quick Svg)
    qt_standard_project_setup()
    qt_add_library(corinzia_ui STATIC
        include/corinzia/ui/EngravingService.hpp src/ui/EngravingService.cpp
        include/corinzia/ui/SvgHitIndex.hpp      src/ui/SvgHitIndex.cpp
        include/corinzia/ui/LayoutAnchors.hpp    src/ui/LayoutAnchors.cpp
        include/corinzia/ui/ScoreCanvas.hpp      src/ui/ScoreCanvas.cpp
    )
    target_link_libraries(corinzia_ui PUBLIC corinzia_core Qt6::Core Qt6::Gui Qt6::Quick Qt6::Svg)
    target_compile_options(corinzia_ui PRIVATE -Wall -Wextra)
endif()

# --------------------------------------------------------------- tests ----
if(CORINZIA_BUILD_TESTS)
    enable_testing()
    add_executable(corinzia_tests
        tests/main.cpp
        tests/test_rt.cpp
        tests/test_model.cpp
        tests/test_engine.cpp
        tests/test_engrave.cpp
        tests/test_storage.cpp
    )
    target_link_libraries(corinzia_tests PRIVATE corinzia_core)
    add_test(NAME corinzia_tests COMMAND corinzia_tests)
    if(CORINZIA_BUILD_UI)
        add_executable(corinzia_ui_tests tests/test_ui.cpp)
        target_link_libraries(corinzia_ui_tests PRIVATE corinzia_ui)
        add_test(NAME corinzia_ui_tests COMMAND corinzia_ui_tests)
        set_tests_properties(corinzia_ui_tests PROPERTIES ENVIRONMENT "QT_QPA_PLATFORM=offscreen")
        if(DEFINED ENV{CORINZIA_LILYPOND})
            set_property(TEST corinzia_tests corinzia_ui_tests APPEND PROPERTY ENVIRONMENT "CORINZIA_LILYPOND=$ENV{CORINZIA_LILYPOND}")
        endif()
    endif()
endif()

# ------------------------------------------------------------- install ----
include(GNUInstallDirs)
install(TARGETS corinzia_core ARCHIVE DESTINATION ${CMAKE_INSTALL_LIBDIR})
if(CORINZIA_BUILD_UI)
    install(TARGETS corinzia_ui ARCHIVE DESTINATION ${CMAKE_INSTALL_LIBDIR})
endif()
install(DIRECTORY include/corinzia DESTINATION ${CMAKE_INSTALL_INCLUDEDIR})
install(FILES schema/project.sql schema/example_project.json DESTINATION ${CMAKE_INSTALL_DATADIR}/corinzia)
