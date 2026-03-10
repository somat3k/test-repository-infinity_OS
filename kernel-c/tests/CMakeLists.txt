# Test harness (header-only interface library)
add_library(test-harness INTERFACE)
target_include_directories(test-harness INTERFACE ${CMAKE_CURRENT_SOURCE_DIR})

# Helper function to register a kernel unit test.
function(add_kernel_test name)
    add_executable(${name} ${name}.c)
    target_link_libraries(${name} PRIVATE infinity-kernel test-harness)
    add_test(NAME ${name} COMMAND ${name})
endfunction()

add_kernel_test(test_kernel)
add_kernel_test(test_memory)
add_kernel_test(test_scheduler)
add_kernel_test(test_ffi)
add_kernel_test(test_service_registry)
add_kernel_test(test_replication)
add_kernel_test(test_trace)
