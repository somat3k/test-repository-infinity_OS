cmake_minimum_required(VERSION 3.16)
project(infinity-kernel LANGUAGES C VERSION 0.1.0)

# ---------------------------------------------------------------------------
# Language standard
# ---------------------------------------------------------------------------
set(CMAKE_C_STANDARD 11)
set(CMAKE_C_EXTENSIONS OFF)
set(CMAKE_C_STANDARD_REQUIRED ON)

# ---------------------------------------------------------------------------
# Compiler flags  (warnings-as-errors — required by README.md)
# ---------------------------------------------------------------------------
if(MSVC)
    add_compile_options(/W4 /WX)
else()
    add_compile_options(-Wall -Wextra -Werror -pedantic)
endif()

# ---------------------------------------------------------------------------
# Dependencies
# ---------------------------------------------------------------------------
find_package(Threads REQUIRED)

# ---------------------------------------------------------------------------
# Library target
# ---------------------------------------------------------------------------
add_library(infinity-kernel STATIC
    src/kernel.c
    src/memory.c
    src/scheduler.c
    src/ffi.c
    src/service_registry.c
    src/replication.c
    src/trace.c
)

target_include_directories(infinity-kernel
    PUBLIC  include
    PRIVATE src
)

target_link_libraries(infinity-kernel PUBLIC Threads::Threads)

# On Linux link libm for clock functions
if(UNIX AND NOT APPLE)
    target_link_libraries(infinity-kernel PRIVATE m)
endif()

# ---------------------------------------------------------------------------
# Install
# ---------------------------------------------------------------------------
install(TARGETS infinity-kernel ARCHIVE DESTINATION lib)
install(DIRECTORY include/infinity DESTINATION include)

# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------
enable_testing()
add_subdirectory(tests)
