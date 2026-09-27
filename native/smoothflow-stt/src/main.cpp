#include "sherpa-onnx/c-api/cxx-api.h"

#include <cstdint>
#include <cstring>
#include <filesystem>
#include <iostream>
#include <limits>
#include <memory>
#include <string>
#include <vector>

#ifdef _WIN32
#include <fcntl.h>
#include <io.h>
#endif

namespace {

constexpr std::int32_t kSampleRate = 16000;

struct Options {
  std::string encoder;
  std::string decoder;
  std::string joiner;
  std::string tokens;
};

std::string ParseOptions(int argc, char** argv, Options& options) {
  for (int i = 1; i < argc; ++i) {
    std::string name = argv[i];
    if (name != "--encoder" && name != "--decoder" && name != "--joiner" &&
        name != "--tokens") {
      return "unknown argument";
    }
    if (i + 1 >= argc || argv[i + 1][0] == '\0') {
      return "missing value for " + name;
    }
    std::string& destination =
        name == "--encoder"   ? options.encoder
        : name == "--decoder" ? options.decoder
        : name == "--joiner"  ? options.joiner
                              : options.tokens;
    if (!destination.empty()) {
      return "duplicate argument: " + name;
    }
    destination = argv[++i];
  }

  if (options.encoder.empty() || options.decoder.empty() ||
      options.joiner.empty() || options.tokens.empty()) {
    return "required arguments: --encoder, --decoder, --joiner, --tokens";
  }
  return {};
}

bool ValidateFile(const std::string& path, const char* name,
                  std::string& error) {
  std::error_code code;
  if (!std::filesystem::is_regular_file(path, code) || code) {
    error = std::string(name) + " must name a regular file";
    return false;
  }
  return true;
}

void EnterBinaryStdin() {
  // MSVC's stdin defaults to text mode, which rewrites CRLF and truncates at
  // 0x1A. Raw float samples must be read as bytes.
#ifdef _WIN32
  _setmode(_fileno(stdin), _O_BINARY);
#endif
  std::ios::sync_with_stdio(false);
}

bool ReadExact(std::size_t count, std::vector<std::uint8_t>& out) {
  out.resize(count);
  std::size_t done = 0;
  while (done < count) {
    std::cin.read(reinterpret_cast<char*>(out.data() + done),
                  static_cast<std::streamsize>(count - done));
    const auto got = static_cast<std::size_t>(std::cin.gcount());
    done += got;
    if (got == 0) {
      return false;
    }
  }
  return true;
}

std::string EscapeJson(const std::string& text) {
  std::string out;
  out.reserve(text.size() + 2);
  for (const unsigned char character : text) {
    switch (character) {
      case '"':  out += "\\\""; break;
      case '\\': out += "\\\\"; break;
      case '\b': out += "\\b";  break;
      case '\f': out += "\\f";  break;
      case '\n': out += "\\n";  break;
      case '\r': out += "\\r";  break;
      case '\t': out += "\\t";  break;
      default:
        if (character < 0x20U) {
          static constexpr char hex[] = "0123456789abcdef";
          out += "\\u00";
          out += hex[character >> 4U];
          out += hex[character & 0x0FU];
        } else {
          out += static_cast<char>(character);
        }
    }
  }
  return out;
}

void WriteLine(const std::string& line) {
  std::cout << line << '\n' << std::flush;
}

void WriteText(const std::string& text) {
  WriteLine("{\"text\":\"" + EscapeJson(text) + "\"}");
}

void WriteError(const std::string& message) {
  WriteLine("{\"error\":\"" + EscapeJson(message) + "\"}");
}

std::vector<float> ToFloatSamples(const std::vector<std::uint8_t>& bytes) {
  std::vector<float> samples(bytes.size() / sizeof(float));
  for (std::size_t i = 0; i < samples.size(); ++i) {
    const auto offset = i * sizeof(float);
    const std::uint32_t bits =
        static_cast<std::uint32_t>(bytes[offset]) |
        (static_cast<std::uint32_t>(bytes[offset + 1]) << 8U) |
        (static_cast<std::uint32_t>(bytes[offset + 2]) << 16U) |
        (static_cast<std::uint32_t>(bytes[offset + 3]) << 24U);
    std::memcpy(&samples[i], &bits, sizeof(bits));
  }
  return samples;
}

}  // namespace

int main(int argc, char** argv) {
  Options options;
  std::string error = ParseOptions(argc, argv, options);
  if (!error.empty()) {
    std::cerr << "error: " << error << '\n'
              << "usage: smoothflow-stt --encoder <path> --decoder <path> "
                 "--joiner <path> --tokens <path>\n"
                 << "protocol: ready line, then repeated "
                    "<uint32 LE sample count><float32 LE samples>; "
                    "one JSON line per request\n";
    return 1;
  }

  if (!ValidateFile(options.encoder, "encoder", error) ||
      !ValidateFile(options.decoder, "decoder", error) ||
      !ValidateFile(options.joiner, "joiner", error) ||
      !ValidateFile(options.tokens, "tokens", error)) {
    std::cerr << "error: " << error << '\n';
    return 1;
  }

  EnterBinaryStdin();

  sherpa_onnx::cxx::OfflineRecognizerConfig config;
  config.feat_config.sample_rate = kSampleRate;
  config.model_config.transducer.encoder = options.encoder;
  config.model_config.transducer.decoder = options.decoder;
  config.model_config.transducer.joiner = options.joiner;
  config.model_config.tokens = options.tokens;
  config.model_config.model_type = "nemo_transducer";
  config.model_config.provider = "cpu";
  config.model_config.num_threads = 2;

  auto recognizer = sherpa_onnx::cxx::OfflineRecognizer::Create(config);
  if (recognizer.Get() == nullptr) {
    std::cerr << "error: failed to initialize recognizer\n";
    return 1;
  }

  WriteLine("{\"ready\":true}");

  std::vector<std::uint8_t> raw;
  std::vector<float> samples;
  for (;;) {
    std::uint8_t header[sizeof(std::uint32_t)];
    if (!ReadExact(sizeof(header), raw)) {
      break;  // clean shutdown on stdin EOF
    }
    std::memcpy(header, raw.data(), sizeof(header));

    const std::uint32_t count = static_cast<std::uint32_t>(header[0]) |
                                (static_cast<std::uint32_t>(header[1]) << 8U) |
                                (static_cast<std::uint32_t>(header[2]) << 16U) |
                                (static_cast<std::uint32_t>(header[3]) << 24U);
    if (count > static_cast<std::uint32_t>(
                     std::numeric_limits<std::int32_t>::max())) {
      WriteError("sample count exceeds the supported maximum");
      continue;
    }
    if (!ReadExact(static_cast<std::size_t>(count) * sizeof(float), raw)) {
      WriteError("truncated audio payload");
      break;
    }

    if (count == 0) {
      WriteText("");
      continue;
    }

    samples = ToFloatSamples(raw);
    auto stream = recognizer.CreateStream();
    if (stream.Get() == nullptr) {
      WriteError("failed to create recognition stream");
      continue;
    }
    stream.AcceptWaveform(kSampleRate, samples.data(),
                          static_cast<std::int32_t>(samples.size()));
    recognizer.Decode(&stream);
    WriteText(recognizer.GetResult(&stream).text);
  }

  return 0;
}
