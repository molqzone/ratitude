cmake_minimum_required(VERSION 3.15)

project(librat_tests C)

add_executable(librat_tests
  test_rat.c
  ../../firmware/src/rat_core.c
  ../../firmware/src/rat_cobs.c
  ../../firmware/src/rat_rtt.c
)

target_include_directories(librat_tests PRIVATE
  ../../firmware/inc
  ../../firmware/src
)

target_compile_definitions(librat_tests PRIVATE RAT_INTERNAL_TEST)
target_compile_features(librat_tests PRIVATE c_std_99)

add_test(NAME librat_tests COMMAND librat_tests)
