#pragma once

#include "dcpdoctor/dcpdoctor.h"
#include <filesystem>
#include <string>
#include <vector>

namespace dcpdoctor
{

/// A suggested fix for a detected issue
struct FixSuggestion
{
  Code related_code;
  std::string description; // Human-readable explanation
  std::string command; // Optional CLI command to fix (empty if manual)
  bool auto_fixable = false; // Whether dcpdoctor can fix it automatically
};

/// Generate fix suggestions for a set of notes
std::vector<FixSuggestion> suggest_fixes(const std::vector<Note>& notes);

/// Apply auto-fixable fixes (returns count of fixes applied)
int apply_fixes(const std::filesystem::path& dcp_dir,
                const std::vector<FixSuggestion>& suggestions);

/// Fix Interop→SMPTE namespaces in XML files
int fix_namespaces(const std::filesystem::path& dcp_dir);

/// Recompute PKL hashes from actual file contents
int fix_pkl_hashes(const std::filesystem::path& dcp_dir);

/// Normalize ContentKind values to canonical lowercase
int fix_content_kind(const std::filesystem::path& dcp_dir);

} // namespace dcpdoctor
