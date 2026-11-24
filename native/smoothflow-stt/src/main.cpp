#include "sherpa-onnx/c-api/cxx-api.h"

#include <cstdint>
#include <cstring>
#include <filesystem>
#include <cmath>
#include <iostream>
#include <limits>
#include <memory>
#include <string>
#include <vector>

#ifdef _WIN32
#include <fcntl.h>
#include <io.h>
#include <windows.h>
#else
#include <poll.h>
#include <unistd.h>
#endif

namespace {

constexpr std::int32_t kSampleRate = 16000;
// Match the Rust side's 30-minute file limit: 30 * 60 * 16 kHz samples. An
// INT32_MAX cap still allowed an 8.6 GB resize before a single payload byte was
// read (NB-15).
constexpr std::uint32_t kMaxSampleCount = 30U * 60U * static_cast<std::uint32_t>(kSampleRate);
// Per-read deadline. A client that stops sending must not pin a helper forever.
constexpr int kReadTimeoutMs = 30000;

struct Options {
  // Shared: every family needs tokens.txt, and the process policy.
  std::string tokens;
  int threads = 0;

  // Transducer (Parakeet TDT / icefall RNNT): the only family with hotwords
  // and a decoding-method choice.
  std::string encoder;
  std::string decoder;
  std::string joiner;
  std::string hotwords;
  float hotwords_score = 1.5F;
  bool hotwords_score_set = false;
  // Empty means "not passed"; resolved to greedy_search at use so the
  // duplicate-argument check sees every destination start empty.
  std::string decoding_method;

  // One-file families (model.onnx + tokens).
  std::string paraformer;
  std::string nemo_ctc;
  std::string zipformer_ctc;
  std::string dolphin;
  std::string sense_voice;
  std::string firered_ctc;

  // Whisper: encoder + decoder, optional language hint for multilingual
  // models.
  std::string whisper_encoder;
  std::string whisper_decoder;
  std::string whisper_language;

  // Moonshine ships two shapes: v1 (uncached+cached decoders) and v2 (one
  // merged decoder).
  std::string moonshine_preprocessor;
  std::string moonshine_encoder;
  std::string moonshine_uncached_decoder;
  std::string moonshine_cached_decoder;
  std::string moonshine_merged_decoder;

  // Canary: encoder + decoder, optional source/target language codes.
  std::string canary_encoder;
  std::string canary_decoder;
  std::string canary_src_lang;
  std::string canary_tgt_lang;

  // FireRed ASR (transducer-shaped): encoder + decoder.
  std::string firered_encoder;
  std::string firered_decoder;
};

/// The flag a given option name writes into. `nullptr` means "unknown".
std::string* DestinationFor(const std::string& name, Options& options) {
  if (name == "--encoder") return &options.encoder;
  if (name == "--decoder") return &options.decoder;
  if (name == "--joiner") return &options.joiner;
  if (name == "--tokens") return &options.tokens;
  if (name == "--paraformer") return &options.paraformer;
  if (name == "--nemo-ctc") return &options.nemo_ctc;
  if (name == "--zipformer-ctc") return &options.zipformer_ctc;
  if (name == "--dolphin") return &options.dolphin;
  if (name == "--sensevoice") return &options.sense_voice;
  if (name == "--firered-ctc") return &options.firered_ctc;
  if (name == "--whisper-encoder") return &options.whisper_encoder;
  if (name == "--whisper-decoder") return &options.whisper_decoder;
  if (name == "--whisper-language") return &options.whisper_language;
  if (name == "--canary-encoder") return &options.canary_encoder;
  if (name == "--canary-decoder") return &options.canary_decoder;
  if (name == "--canary-src-lang") return &options.canary_src_lang;
  if (name == "--canary-tgt-lang") return &options.canary_tgt_lang;
  if (name == "--firered-encoder") return &options.firered_encoder;
  if (name == "--firered-decoder") return &options.firered_decoder;
  if (name == "--moonshine-preprocessor") return &options.moonshine_preprocessor;
  if (name == "--moonshine-encoder") return &options.moonshine_encoder;
  if (name == "--moonshine-uncached-decoder") return &options.moonshine_uncached_decoder;
  if (name == "--moonshine-cached-decoder") return &options.moonshine_cached_decoder;
  if (name == "--moonshine-merged-decoder") return &options.moonshine_merged_decoder;
  if (name == "--hotwords") return &options.hotwords;
  if (name == "--decoding-method") return &options.decoding_method;
  return nullptr;
}

/// One recognisable model family: the flags that must all be given, in order.
/// sherpa-onnx loads whichever family's config fields are set ("Configure
/// exactly one model family", cxx-api.h), so the helper's job is to check that
/// exactly one was configured and reject half-specified bundles.
struct FamilyDef {
  const char* id;
  std::vector<std::pair<const char*, const std::string*>> args;
};

/// The `--tokens` value is shared and validated on its own, so families list
/// only their model files.
std::vector<FamilyDef> BuildFamilies(const Options& options) {
  return {
      {"transducer",
       {{"--encoder", &options.encoder},
        {"--decoder", &options.decoder},
        {"--joiner", &options.joiner}}},
      {"whisper",
       {{"--whisper-encoder", &options.whisper_encoder},
        {"--whisper-decoder", &options.whisper_decoder}}},
      {"moonshine",
       {{"--moonshine-preprocessor", &options.moonshine_preprocessor},
        {"--moonshine-encoder", &options.moonshine_encoder},
        {"--moonshine-uncached-decoder", &options.moonshine_uncached_decoder},
        {"--moonshine-cached-decoder", &options.moonshine_cached_decoder}}},
      {"moonshine-v2",
       {{"--moonshine-preprocessor", &options.moonshine_preprocessor},
        {"--moonshine-encoder", &options.moonshine_encoder},
        {"--moonshine-merged-decoder", &options.moonshine_merged_decoder}}},
      {"canary",
       {{"--canary-encoder", &options.canary_encoder},
        {"--canary-decoder", &options.canary_decoder}}},
      {"fire_red_asr",
       {{"--firered-encoder", &options.firered_encoder},
        {"--firered-decoder", &options.firered_decoder}}},
      {"sense_voice", {{"--sensevoice", &options.sense_voice}}},
      {"paraformer", {{"--paraformer", &options.paraformer}}},
      {"nemo_ctc", {{"--nemo-ctc", &options.nemo_ctc}}},
      {"zipformer2_ctc", {{"--zipformer-ctc", &options.zipformer_ctc}}},
      {"fire_red_asr_ctc", {{"--firered-ctc", &options.firered_ctc}}},
      {"dolphin", {{"--dolphin", &options.dolphin}}},
  };
}

/// A flag may only be used with its own model family. sherpa-onnx silently
/// ignores config fields from a different family, so a typo used to produce a
/// recognizer that loaded but ignored what the caller asked for (NB-30).
struct FlagOwner {
  const char* flag;
  const char* family;
  const char* also;  // a second family that legitimately shares this flag
};

const std::vector<FlagOwner>& FlagOwners() {
  static const std::vector<FlagOwner> owners = {
      {"--encoder", "transducer", nullptr},
      {"--decoder", "transducer", nullptr},
      {"--joiner", "transducer", nullptr},
      {"--hotwords", "transducer", nullptr},
      {"--hotwords-score", "transducer", nullptr},
      {"--decoding-method", "transducer", nullptr},
      {"--whisper-encoder", "whisper", nullptr},
      {"--whisper-decoder", "whisper", nullptr},
      {"--whisper-language", "whisper", nullptr},
      {"--moonshine-preprocessor", "moonshine", "moonshine-v2"},
      {"--moonshine-encoder", "moonshine", "moonshine-v2"},
      {"--moonshine-uncached-decoder", "moonshine", nullptr},
      {"--moonshine-cached-decoder", "moonshine", nullptr},
      {"--moonshine-merged-decoder", "moonshine-v2", nullptr},
      {"--canary-encoder", "canary", nullptr},
      {"--canary-decoder", "canary", nullptr},
      {"--canary-src-lang", "canary", nullptr},
      {"--canary-tgt-lang", "canary", nullptr},
      {"--firered-encoder", "fire_red_asr", nullptr},
      {"--firered-decoder", "fire_red_asr", nullptr},
      {"--firered-ctc", "fire_red_asr_ctc", nullptr},
      {"--sensevoice", "sense_voice", nullptr},
      {"--paraformer", "paraformer", nullptr},
      {"--nemo-ctc", "nemo_ctc", nullptr},
      {"--zipformer-ctc", "zipformer2_ctc", nullptr},
      {"--dolphin", "dolphin", nullptr},
  };
  return owners;
}

bool FlagWasSet(const std::string& flag, const Options& options) {
  if (flag == "--hotwords-score") {
    return options.hotwords_score_set;
  }
  const std::string* value = DestinationFor(flag, options);
  return value != nullptr && !value->empty();
}

std::string ParseOptions(int argc, char** argv, Options& options) {
  for (int i = 1; i < argc; ++i) {
    const std::string name = argv[i];
    if (name != "--threads" && DestinationFor(name, options) == nullptr &&
        name != "--hotwords-score") {
      return "unknown argument";
    }
    if (i + 1 >= argc || argv[i + 1][0] == '\0') {
      return "missing value for " + name;
    }
    if (name == "--threads") {
      if (options.threads != 0) {
        return "duplicate argument: " + name;
      }
      try {
        const int threads = std::stoi(argv[++i]);
        // Above 64 the value is a mistake, not a request; the encoder would
        // just spend its time context-switching.
        if (threads < 1 || threads > 64) {
          return "invalid value for " + name;
        }
        options.threads = threads;
      } catch (const std::exception&) {
        return "invalid value for " + name;
      }
      continue;
    }
    if (name == "--hotwords-score") {
      if (options.hotwords_score_set) {
        return "duplicate argument: " + name;
      }
      try {
        std::size_t parsed = 0;
        const float score = std::stof(argv[++i], &parsed);
        // std::stof happily returns nan/inf, and trailing garbage is accepted
        // unless the whole token is consumed. A non-finite boost makes the
        // decoder config nonsense (NB-16).
        if (parsed != std::string(argv[i]).size() || !std::isfinite(score) ||
            score < 0.0F || score > 100.0F) {
          return "invalid value for " + name + " (expected a finite number from 0 to 100)";
        }
        options.hotwords_score = score;
        options.hotwords_score_set = true;
      } catch (const std::exception&) {
        return "invalid value for " + name;
      }
      continue;
    }
    std::string* destination = DestinationFor(name, options);
    if (!destination->empty()) {
      return "duplicate argument: " + name;
    }
    *destination = argv[++i];
  }

  if (options.tokens.empty()) {
    return "required arguments: --tokens (plus the flags of exactly one "
           "model family)";
  }
  return {};
}

/// Exactly one family must be fully specified. On success `family` holds its
/// id; a half-specified or ambiguous invocation names the offending flags.
std::string ValidateFamilies(const std::vector<FamilyDef>& families,
                             Options& options, std::string& family) {
  std::string configured;
  for (const FamilyDef& def : families) {
    std::size_t set = 0;
    std::string given;
    for (const auto& [flag, value] : def.args) {
      if (value->empty()) {
        continue;
      }
      ++set;
      if (!given.empty()) {
        given += ", ";
      }
      given += flag;
    }
    if (set == 0) {
      continue;
    }
    if (set == def.args.size()) {
      if (!configured.empty()) {
        return "only one model family may be configured, found both " +
               configured + " and " + def.id;
      }
      configured = def.id;
      continue;
    }
    std::string required;
    for (const auto& [flag, value] : def.args) {
      if (!required.empty()) {
        required += " ";
      }
      required += flag;
    }
    return "the " + std::string(def.id) + " family needs all of: " + required +
           " (got " + (set ? given : "none") + ")";
  }

  if (configured.empty()) {
    return "exactly one model family must be configured; see usage below";
  }
  // Every flag that was supplied must belong to the selected family. This
  // subsumes the old transducer-only hotwords/decoding-method check: a
  // --whisper-language passed to a transducer (or its reverse) is now rejected
  // instead of being silently ignored (NB-30).
  for (const FlagOwner& owner : FlagOwners()) {
    const std::string flag(owner.flag);
    if (!FlagWasSet(flag, options)) {
      continue;
    }
    if (configured == owner.family ||
        (owner.also != nullptr && configured == owner.also)) {
      continue;
    }
    return flag + " is not valid with the " + std::string(configured) +
           " family (it belongs to " + owner.family + ")";
  }
  family = configured;
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

/// Every path that was given must exist. `--tokens` is always required; the
/// configured family's files are all set by then, and hotwords keep their own
/// optional check.
bool ValidatePaths(const std::vector<FamilyDef>& families,
                   const Options& options, std::string& error) {
  if (!ValidateFile(options.tokens, "--tokens", error)) {
    return false;
  }
  for (const FamilyDef& def : families) {
    for (const auto& [flag, value] : def.args) {
      if (value->empty()) {
        continue;
      }
      if (!ValidateFile(*value, flag, error)) {
        return false;
      }
    }
  }
  if (!options.hotwords.empty() &&
      !ValidateFile(options.hotwords, "--hotwords", error)) {
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

/// Wait for stdin to have data. A client that sends a header and then stops
/// must not pin the helper process forever (NB-15).
bool WaitForStdin() {
#ifdef _WIN32
  const HANDLE input = GetStdHandle(STD_INPUT_HANDLE);
  if (input == INVALID_HANDLE_VALUE) {
    return false;
  }
  return WaitForSingleObject(input, static_cast<DWORD>(kReadTimeoutMs)) ==
         WAIT_OBJECT_0;
#else
  pollfd descriptor{};
  descriptor.fd = STDIN_FILENO;
  descriptor.events = POLLIN;
  const int ready = poll(&descriptor, 1, kReadTimeoutMs);
  return ready > 0;
#endif
}

/// One bounded read from stdin, bypassing std::cin's buffer so the readiness
/// check above and the actual read see the same bytes.
bool ReadSomeStdin(std::uint8_t* destination, std::size_t length,
                   std::size_t& got) {
#ifdef _WIN32
  DWORD bytes = 0;
  if (!ReadFile(GetStdHandle(STD_INPUT_HANDLE), destination,
                static_cast<DWORD>(length), &bytes, nullptr)) {
    return false;
  }
  got = static_cast<std::size_t>(bytes);
#else
  const ssize_t bytes = read(STDIN_FILENO, destination, length);
  if (bytes < 0) {
    return false;
  }
  got = static_cast<std::size_t>(bytes);
#endif
  return true;
}

enum class ReadResult { kOk, kEof, kTimeout, kError };

ReadResult ReadExact(std::size_t count, std::vector<std::uint8_t>& out) {
  out.resize(count);
  std::size_t done = 0;
  while (done < count) {
    if (!WaitForStdin()) {
      out.clear();
      return ReadResult::kTimeout;
    }
    std::size_t got = 0;
    if (!ReadSomeStdin(out.data() + done, count - done, got)) {
      out.clear();
      return ReadResult::kError;
    }
    if (got == 0) {
      out.clear();
      return ReadResult::kEof;
    }
    done += got;
  }
  return ReadResult::kOk;
}

/// Length of the well-formed UTF-8 sequence at `text[offset]`, or 0 when the
/// bytes are invalid. Rust's `read_line` rejects invalid UTF-8 and tears the
/// session down, so a bad recogniser output must never reach the JSON line
/// (NB-17).
std::size_t Utf8SequenceLength(const std::string& text, std::size_t offset) {
  const auto lead = static_cast<unsigned char>(text[offset]);
  if (lead < 0x80U) {
    return 1;
  }
  std::size_t continuation = 0;
  unsigned char low = 0x80U;
  unsigned char high = 0xbfU;
  if (lead >= 0xc2U && lead <= 0xdfU) {
    continuation = 1;
  } else if (lead == 0xe0U) {
    continuation = 2;
    low = 0xa0U;  // no overlong encodings
  } else if (lead >= 0xe1U && lead <= 0xecU) {
    continuation = 2;
  } else if (lead == 0xedU) {
    continuation = 2;
    high = 0x9fU;  // no UTF-16 surrogates
  } else if (lead >= 0xeeU && lead <= 0xefU) {
    continuation = 2;
  } else if (lead == 0xf0U) {
    continuation = 3;
    low = 0x90U;
  } else if (lead >= 0xf1U && lead <= 0xf3U) {
    continuation = 3;
  } else if (lead == 0xf4U) {
    continuation = 3;
    high = 0x8fU;
  } else {
    return 0;
  }
  if (text.size() - offset <= continuation) {
    return 0;
  }
  for (std::size_t i = 1; i <= continuation; ++i) {
    const auto byte = static_cast<unsigned char>(text[offset + i]);
    const unsigned char minimum = i == 1 ? low : 0x80U;
    const unsigned char maximum = i == 1 ? high : 0xbfU;
    if (byte < minimum || byte > maximum) {
      return 0;
    }
  }
  return continuation + 1;
}

std::string EscapeJson(const std::string& text) {
  std::string out;
  out.reserve(text.size() + 2);
  for (std::size_t i = 0; i < text.size();) {
    const std::size_t length = Utf8SequenceLength(text, i);
    if (length == 0) {
      // Replace just the offending byte and keep scanning: one bad sequence
      // must not cost the user the rest of their transcript.
      out += "\xef\xbf\xbd";  // U+FFFD
      ++i;
      continue;
    }
    if (length > 1) {
      out.append(text, i, length);
      i += length;
      continue;
    }
    const auto character = static_cast<unsigned char>(text[i++]);
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
  const std::vector<FamilyDef> families = BuildFamilies(options);
  std::string family;
  std::string error = ParseOptions(argc, argv, options);
  if (error.empty()) {
    error = ValidateFamilies(families, options, family);
  }
  if (error.empty()) {
    ValidatePaths(families, options, error);
  }
  if (!error.empty()) {
    std::cerr << "error: " << error << '\n'
              << "usage: smoothflow-stt --tokens <path> [--threads <int>]\n"
                 "  plus the flags of exactly ONE model family:\n"
                 "    transducer:    --encoder <p> --decoder <p> --joiner <p>\n"
                 "                   [--hotwords <p>] [--hotwords-score <f>]\n"
                 "                   [--decoding-method <greedy_search|modified_beam_search>]\n"
                 "    whisper:       --whisper-encoder <p> --whisper-decoder <p> [--whisper-language <code>]\n"
                 "    moonshine:     --moonshine-preprocessor <p> --moonshine-encoder <p>\n"
                 "                   --moonshine-uncached-decoder <p> --moonshine-cached-decoder <p>\n"
                 "    moonshine-v2:  --moonshine-preprocessor <p> --moonshine-encoder <p>\n"
                 "                   --moonshine-merged-decoder <p>\n"
                 "    canary:        --canary-encoder <p> --canary-decoder <p> [--canary-src-lang <c>] [--canary-tgt-lang <c>]\n"
                 "    fire_red_asr:  --firered-encoder <p> --firered-decoder <p>\n"
                 "    sense_voice:   --sensevoice <p>\n"
                 "    paraformer:    --paraformer <p>\n"
                 "    nemo_ctc:      --nemo-ctc <p>\n"
                 "    zipformer2_ctc:--zipformer-ctc <p>\n"
                 "    fire_red_asr_ctc: --firered-ctc <p>\n"
                 "    dolphin:       --dolphin <p>\n"
                 "protocol: ready line, then repeated "
                 "<uint32 LE sample count><float32 LE samples>; "
                 "one JSON line per request\n";
    return 1;
  }

  EnterBinaryStdin();

  sherpa_onnx::cxx::OfflineRecognizerConfig config;
  config.feat_config.sample_rate = kSampleRate;
  config.model_config.tokens = options.tokens;
  config.model_config.provider = "cpu";
  config.model_config.num_threads = options.threads > 0 ? options.threads : 2;
  // model_type stays empty: sherpa-onnx picks the impl from whichever family
  // field is set (or, for transducers, from the encoder's own ONNX metadata),
  // so the helper never needs a rebuild when a new family is added upstream.
  if (family == "transducer") {
    config.model_config.transducer.encoder = options.encoder;
    config.model_config.transducer.decoder = options.decoder;
    config.model_config.transducer.joiner = options.joiner;
    // Absent --hotwords leaves hotwords_file at its empty default, so the
    // recognizer is configured exactly as it was before this flag existed.
    if (!options.hotwords.empty()) {
      config.hotwords_file = options.hotwords;
      config.hotwords_score = options.hotwords_score;
      // sherpa-onnx rejects hotwords under greedy_search outright ("Please use
      // --decoding-method=modified_beam_search if you provide
      // --hotwords-file"), so hotwords imply beam search rather than erroring.
      // Transducer hotwords reach the decoder through the modified-beam-search
      // context graph; there is no greedy path that can apply them.
      if (options.decoding_method == "greedy_search" ||
          options.decoding_method.empty()) {
        options.decoding_method = "modified_beam_search";
      }
    }
    if (options.decoding_method.empty()) {
      options.decoding_method = "greedy_search";
    }
    config.decoding_method = options.decoding_method;
  } else {
    // The transducer-only flags were rejected in ValidateFamilies, so the
    // default search serves every other family.
    config.decoding_method = "greedy_search";
    if (family == "whisper") {
      config.model_config.whisper.encoder = options.whisper_encoder;
      config.model_config.whisper.decoder = options.whisper_decoder;
      config.model_config.whisper.language = options.whisper_language;
    } else if (family == "canary") {
      config.model_config.canary.encoder = options.canary_encoder;
      config.model_config.canary.decoder = options.canary_decoder;
      config.model_config.canary.src_lang = options.canary_src_lang;
      config.model_config.canary.tgt_lang = options.canary_tgt_lang;
    } else if (family == "fire_red_asr") {
      config.model_config.fire_red_asr.encoder = options.firered_encoder;
      config.model_config.fire_red_asr.decoder = options.firered_decoder;
    } else if (family == "sense_voice") {
      config.model_config.sense_voice.model = options.sense_voice;
    } else if (family == "paraformer") {
      config.model_config.paraformer.model = options.paraformer;
    } else if (family == "nemo_ctc") {
      config.model_config.nemo_ctc.model = options.nemo_ctc;
    } else if (family == "zipformer2_ctc") {
      config.model_config.zipformer_ctc.model = options.zipformer_ctc;
    } else if (family == "fire_red_asr_ctc") {
      config.model_config.fire_red_asr_ctc.model = options.firered_ctc;
    } else if (family == "dolphin") {
      config.model_config.dolphin.model = options.dolphin;
    } else if (family == "moonshine" || family == "moonshine-v2") {
      config.model_config.moonshine.preprocessor = options.moonshine_preprocessor;
      config.model_config.moonshine.encoder = options.moonshine_encoder;
      config.model_config.moonshine.uncached_decoder =
          options.moonshine_uncached_decoder;
      config.model_config.moonshine.cached_decoder =
          options.moonshine_cached_decoder;
      config.model_config.moonshine.merged_decoder =
          options.moonshine_merged_decoder;
    }
  }

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
    const ReadResult header_read = ReadExact(sizeof(header), raw);
    if (header_read != ReadResult::kOk) {
      if (header_read == ReadResult::kTimeout) {
        WriteError("timed out waiting for audio");
      }
      break;  // EOF, timeout or I/O error: no further request can be trusted
    }
    std::memcpy(header, raw.data(), sizeof(header));

    const std::uint32_t count = static_cast<std::uint32_t>(header[0]) |
                                (static_cast<std::uint32_t>(header[1]) << 8U) |
                                (static_cast<std::uint32_t>(header[2]) << 16U) |
                                (static_cast<std::uint32_t>(header[3]) << 24U);
    if (count > kMaxSampleCount) {
      WriteError("sample count exceeds the 30-minute audio limit");
      continue;
    }
    const ReadResult body_read =
        ReadExact(static_cast<std::size_t>(count) * sizeof(float), raw);
    if (body_read != ReadResult::kOk) {
      if (body_read == ReadResult::kTimeout) {
        WriteError("timed out waiting for audio");
      } else {
        WriteError("truncated audio payload");
      }
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
