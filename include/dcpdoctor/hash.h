#pragma once

#include <cstdint>
#include <filesystem>
#include <functional>
#include <optional>
#include <string>

namespace dcpdoctor
{

/// Compute SHA-1 hash of a file, returned as base64
std::optional<std::string> sha1_base64(const std::filesystem::path& file);

/// Compute SHA-1 hash with progress callback: (bytes_done, bytes_total)
std::optional<std::string>
    sha1_base64(const std::filesystem::path& file,
                const std::function<void(std::uintmax_t, std::uintmax_t)>& progress);

/// Compute SHA-1 hash of a file, returned as hex
std::optional<std::string> sha1_hex(const std::filesystem::path& file);

} // namespace dcpdoctor
