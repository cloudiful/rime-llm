cmake_minimum_required(VERSION 3.16)
project(rime-llm-predict LANGUAGES CXX)

if(NOT APPLE)
  message(FATAL_ERROR "The native plugin currently targets macOS Squirrel only")
endif()

set(RIME_SOURCE_DIR "" CACHE PATH "librime source checkout with private headers")
set(RIME_LIBRARY "/Library/Input Methods/Squirrel.app/Contents/Frameworks/librime.1.dylib"
    CACHE FILEPATH "librime dylib used by Squirrel")

if(NOT EXISTS "${RIME_SOURCE_DIR}/src/rime/processor.h")
  message(FATAL_ERROR
    "RIME_SOURCE_DIR must point at the matching librime source checkout (1.16.x)")
endif()
if(NOT EXISTS "${RIME_LIBRARY}")
  message(FATAL_ERROR "RIME_LIBRARY was not found: ${RIME_LIBRARY}")
endif()

find_path(BOOST_INCLUDE_DIR boost/signals2/connection.hpp
  PATHS /opt/homebrew/include /usr/local/include
  REQUIRED
)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)

set(RIME_DATA_DIR "rime-data")
set(RIME_PLUGINS_DIR "rime-plugins")
set(RIME_ENABLE_LOGGING OFF)
file(MAKE_DIRECTORY "${CMAKE_CURRENT_BINARY_DIR}/rime")
configure_file(
  "${RIME_SOURCE_DIR}/src/rime/build_config.h.in"
  "${CMAKE_CURRENT_BINARY_DIR}/rime/build_config.h"
)

set(PLUGIN_SOURCES
  src/http_client.cc
  src/llm_filter.cc
  src/llm_json.cc
  src/llm_module.cc
  src/llm_processor.cc
  src/rime_candidate_graph.cc
  src/llm_session.cc
  src/llm_session_candidates.cc
  src/llm_translator.cc
  src/llm_worker.cc
)

add_library(rime-llm-predict SHARED ${PLUGIN_SOURCES})
target_include_directories(rime-llm-predict PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/include
  ${CMAKE_CURRENT_SOURCE_DIR}/src
  ${CMAKE_CURRENT_BINARY_DIR}
  ${RIME_SOURCE_DIR}/src
  ${RIME_SOURCE_DIR}/include
  ${BOOST_INCLUDE_DIR}
)
target_link_libraries(rime-llm-predict PRIVATE ${RIME_LIBRARY})
target_link_options(rime-llm-predict PRIVATE -Wl,-undefined,dynamic_lookup)
set_target_properties(rime-llm-predict PROPERTIES
  OUTPUT_NAME "rime-llm-predict"
  INSTALL_NAME_DIR "@rpath"
  LIBRARY_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/rime-plugins"
)

include(CTest)
if(BUILD_TESTING)
  add_executable(rime-llm-predict-test
    test/llm_json_test.cc
    src/llm_json.cc
  )
  target_include_directories(rime-llm-predict-test PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/include
    ${CMAKE_CURRENT_SOURCE_DIR}/src
  )
  add_test(NAME rime-llm-predict-json COMMAND rime-llm-predict-test)
endif()

install(TARGETS rime-llm-predict
  LIBRARY DESTINATION rime-plugins
)
