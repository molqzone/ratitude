cmake_minimum_required(VERSION 3.15)

project(librat C)

add_library(librat STATIC
  src/rat_core.c
  src/rat_cobs.c
)

target_include_directories(librat PUBLIC
  ${CMAKE_CURRENT_LIST_DIR}/inc
)

target_compile_features(librat PUBLIC c_std_99)

option(RAT_BUILD_TESTS "Build librat tests" OFF)
if (RAT_BUILD_TESTS)
  enable_testing()
  add_subdirectory(../tests/rat ${CMAKE_BINARY_DIR}/rat_tests)
endif()
