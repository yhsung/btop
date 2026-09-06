# SPDX-License-Identifier: Apache-2.0

include(FetchContent)
FetchContent_Declare(
  googletest
  GIT_REPOSITORY https://github.com/google/googletest.git
  GIT_TAG v1.17.0
  FIND_PACKAGE_ARGS NAMES GTest
)
FetchContent_MakeAvailable(googletest)

add_library(libbtop_test)
target_include_directories(libbtop_test PUBLIC ${PROJECT_SOURCE_DIR}/src)
target_link_libraries(libbtop_test libbtop GTest::gtest_main)

add_executable(btop_test cpu_names.cpp tools.cpp)
target_link_libraries(btop_test libbtop_test)

include(GoogleTest)
gtest_discover_tests(btop_test)

# Golden-output harness for the draw (M3) port. Links libbtop directly (not
# libbtop_test: that pulls in GTest::gtest_main whose main() would clash with
# the harness main) plus the src include dir libbtop_test would provide.
add_executable(draw_golden draw_golden.cpp)
target_include_directories(draw_golden PRIVATE ${PROJECT_SOURCE_DIR}/src)
target_link_libraries(draw_golden libbtop)
