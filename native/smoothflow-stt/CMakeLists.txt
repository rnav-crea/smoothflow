cmake_minimum_required(VERSION 3.15 FATAL_ERROR)

project(smoothflow-stt LANGUAGES CXX)

if(NOT DEFINED SHERPA_ONNX_SOURCE_DIR OR SHERPA_ONNX_SOURCE_DIR STREQUAL "")
  message(FATAL_ERROR "SHERPA_ONNX_SOURCE_DIR must point to the sherpa-onnx v1.13.8 source tree")
endif()

if(NOT IS_DIRECTORY "${SHERPA_ONNX_SOURCE_DIR}" OR NOT EXISTS "${SHERPA_ONNX_SOURCE_DIR}/CMakeLists.txt")
  message(FATAL_ERROR "SHERPA_ONNX_SOURCE_DIR must contain the sherpa-onnx v1.13.8 CMakeLists.txt")
endif()

file(READ "${SHERPA_ONNX_SOURCE_DIR}/CMakeLists.txt" SHERPA_ONNX_ROOT_CMAKE)
if(NOT SHERPA_ONNX_ROOT_CMAKE MATCHES "set\\(SHERPA_ONNX_VERSION \"1\\.13\\.8\"")
  message(FATAL_ERROR "SHERPA_ONNX_SOURCE_DIR must contain sherpa-onnx v1.13.8")
endif()

# Headers come from SHERPA_ONNX_SOURCE_DIR, code from the official prebuilt
# static bundle for the same pinned version. Building sherpa-onnx from source
# produces identical objects and costs hours.
if(NOT DEFINED SHERPA_ONNX_PREBUILT_DIR OR SHERPA_ONNX_PREBUILT_DIR STREQUAL "")
  message(FATAL_ERROR "SHERPA_ONNX_PREBUILT_DIR must point to the extracted sherpa-onnx v1.13.8 win-x64 static MT Release no-tts lib bundle")
endif()

file(GLOB SHERPA_ONNX_PREBUILT_LIBS "${SHERPA_ONNX_PREBUILT_DIR}/lib/*.lib")
if(NOT SHERPA_ONNX_PREBUILT_LIBS)
  message(FATAL_ERROR "SHERPA_ONNX_PREBUILT_DIR contains no lib/*.lib")
endif()
message(STATUS "Prebuilt sherpa-onnx libs: ${SHERPA_ONNX_PREBUILT_LIBS}")

# ponytail: link every bundled lib rather than transcribing sherpa-onnx's
# internal link graph; link.exe resolves symbols across all inputs in one pass.
add_executable(smoothflow-stt src/main.cpp)
target_compile_features(smoothflow-stt PRIVATE cxx_std_17)
target_include_directories(smoothflow-stt PRIVATE "${SHERPA_ONNX_SOURCE_DIR}")
target_link_libraries(smoothflow-stt PRIVATE ${SHERPA_ONNX_PREBUILT_LIBS})

# The prebuilt bundle is /MT, so the helper must be /MT too or the CRTs mismatch.
set_property(TARGET smoothflow-stt PROPERTY
  MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>")

install(TARGETS smoothflow-stt RUNTIME DESTINATION bin)
