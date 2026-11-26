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
  message(FATAL_ERROR "SHERPA_ONNX_PREBUILT_DIR must point to the extracted sherpa-onnx v1.13.8 static no-tts lib bundle for this platform")
endif()

# Windows ships .lib, Linux and macOS ship .a. Both are the same static bundle.
file(GLOB SHERPA_ONNX_PREBUILT_LIBS "${SHERPA_ONNX_PREBUILT_DIR}/lib/*.lib" "${SHERPA_ONNX_PREBUILT_DIR}/lib/*.a")
if(NOT SHERPA_ONNX_PREBUILT_LIBS)
  message(FATAL_ERROR "SHERPA_ONNX_PREBUILT_DIR contains no lib/*.lib or lib/*.a")
endif()
message(STATUS "Prebuilt sherpa-onnx libs: ${SHERPA_ONNX_PREBUILT_LIBS}")

# ponytail: link every bundled lib rather than transcribing sherpa-onnx's
# internal link graph; link.exe resolves symbols across all inputs in one pass.
add_executable(smoothflow-stt src/main.cpp)
target_compile_features(smoothflow-stt PRIVATE cxx_std_17)
target_include_directories(smoothflow-stt PRIVATE "${SHERPA_ONNX_SOURCE_DIR}")
if(MSVC)
  target_link_libraries(smoothflow-stt PRIVATE ${SHERPA_ONNX_PREBUILT_LIBS})
elseif(APPLE)
  # ld64 has no --start-group; the cross-archive references resolve on the
  # macOS runners without it.
  target_link_libraries(smoothflow-stt PRIVATE ${SHERPA_ONNX_PREBUILT_LIBS})
else()
  # GNU ld scans each archive once, in order. libsherpa-onnx-cxx-api.a
  # references C-API symbols (e.g. SherpaOnnxDestroyOfflineDiacritization)
  # that live in libsherpa-onnx-c-api.a, which the glob can list earlier —
  # a rescan group makes the references resolve regardless of order.
  target_link_libraries(smoothflow-stt PRIVATE
    "-Wl,--start-group" ${SHERPA_ONNX_PREBUILT_LIBS} "-Wl,--end-group")
endif()

# The static bundle is /MT, so the helper must be /MT too or the CRTs mismatch.
# The property is MSVC-only; GCC/Clang have no equivalent and do not need one.
if(MSVC)
  set_property(TARGET smoothflow-stt PROPERTY
    MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>")
endif()

# Static onnxruntime still needs the platform's thread and dynamic-loading libs.
find_package(Threads REQUIRED)
target_link_libraries(smoothflow-stt PRIVATE Threads::Threads ${CMAKE_DL_LIBS})

if(APPLE)
  # onnxruntime's static archive references Accelerate for its CPU kernels,
  # Foundation for NSLog (apple_log_sink.mm.o) and CoreFoundation for
  # __CFConstantStringClassReference plus absl's cctz local_time_zone()
  # (_CFTimeZoneGetName). Missing Foundation/CoreFoundation fails the link with
  # "symbol(s) not found for architecture arm64/x86_64".
  target_link_libraries(smoothflow-stt PRIVATE
    "-framework Accelerate"
    "-framework Foundation"
    "-framework CoreFoundation")
endif()

install(TARGETS smoothflow-stt RUNTIME DESTINATION bin)
