#include <openssl/evp.h>
#include <openssl/bio.h>
#include <openssl/buffer.h>
#include <fstream>
#include <functional>
#include <vector>
#include <array>
#include <format>

#include "dcpdoctor/hash.h"

namespace dcpdoctor
{

static std::string base64_encode(const unsigned char* data, size_t len)
{
  BIO* b64 = BIO_new(BIO_f_base64());
  BIO* bmem = BIO_new(BIO_s_mem());
  b64 = BIO_push(b64, bmem);
  BIO_set_flags(b64, BIO_FLAGS_BASE64_NO_NL);
  BIO_write(b64, data, static_cast<int>(len));
  BIO_flush(b64);

  BUF_MEM* bptr;
  BIO_get_mem_ptr(b64, &bptr);
  std::string result(bptr->data, bptr->length);
  BIO_free_all(b64);
  return result;
}

std::optional<std::string> sha1_base64(const std::filesystem::path& file)
{
  std::ifstream in(file, std::ios::binary);
  if(!in)
    return std::nullopt;

  EVP_MD_CTX* ctx = EVP_MD_CTX_new();
  if(!ctx)
    return std::nullopt;

  if(EVP_DigestInit_ex(ctx, EVP_sha1(), nullptr) != 1)
  {
    EVP_MD_CTX_free(ctx);
    return std::nullopt;
  }

  std::array<char, 65536> buf;
  while(in.read(buf.data(), buf.size()) || in.gcount() > 0)
  {
    if(EVP_DigestUpdate(ctx, buf.data(), static_cast<size_t>(in.gcount())) != 1)
    {
      EVP_MD_CTX_free(ctx);
      return std::nullopt;
    }
  }

  unsigned char hash[EVP_MAX_MD_SIZE];
  unsigned int hash_len = 0;
  if(EVP_DigestFinal_ex(ctx, hash, &hash_len) != 1)
  {
    EVP_MD_CTX_free(ctx);
    return std::nullopt;
  }
  EVP_MD_CTX_free(ctx);

  return base64_encode(hash, hash_len);
}

std::optional<std::string>
    sha1_base64(const std::filesystem::path& file,
                const std::function<void(std::uintmax_t, std::uintmax_t)>& progress)
{
  std::ifstream in(file, std::ios::binary);
  if(!in)
    return std::nullopt;

  auto file_size = std::filesystem::file_size(file);

  EVP_MD_CTX* ctx = EVP_MD_CTX_new();
  if(!ctx)
    return std::nullopt;

  if(EVP_DigestInit_ex(ctx, EVP_sha1(), nullptr) != 1)
  {
    EVP_MD_CTX_free(ctx);
    return std::nullopt;
  }

  std::array<char, 65536> buf;
  std::uintmax_t bytes_done = 0;
  while(in.read(buf.data(), buf.size()) || in.gcount() > 0)
  {
    auto count = static_cast<size_t>(in.gcount());
    if(EVP_DigestUpdate(ctx, buf.data(), count) != 1)
    {
      EVP_MD_CTX_free(ctx);
      return std::nullopt;
    }
    bytes_done += count;
    if(progress)
      progress(bytes_done, file_size);
  }

  unsigned char hash[EVP_MAX_MD_SIZE];
  unsigned int hash_len = 0;
  if(EVP_DigestFinal_ex(ctx, hash, &hash_len) != 1)
  {
    EVP_MD_CTX_free(ctx);
    return std::nullopt;
  }
  EVP_MD_CTX_free(ctx);

  return base64_encode(hash, hash_len);
}

std::optional<std::string> sha1_hex(const std::filesystem::path& file)
{
  std::ifstream in(file, std::ios::binary);
  if(!in)
    return std::nullopt;

  EVP_MD_CTX* ctx = EVP_MD_CTX_new();
  if(!ctx)
    return std::nullopt;

  if(EVP_DigestInit_ex(ctx, EVP_sha1(), nullptr) != 1)
  {
    EVP_MD_CTX_free(ctx);
    return std::nullopt;
  }

  std::array<char, 65536> buf;
  while(in.read(buf.data(), buf.size()) || in.gcount() > 0)
  {
    if(EVP_DigestUpdate(ctx, buf.data(), static_cast<size_t>(in.gcount())) != 1)
    {
      EVP_MD_CTX_free(ctx);
      return std::nullopt;
    }
  }

  unsigned char hash[EVP_MAX_MD_SIZE];
  unsigned int hash_len = 0;
  if(EVP_DigestFinal_ex(ctx, hash, &hash_len) != 1)
  {
    EVP_MD_CTX_free(ctx);
    return std::nullopt;
  }
  EVP_MD_CTX_free(ctx);

  std::string result;
  result.reserve(hash_len * 2);
  for(unsigned int i = 0; i < hash_len; ++i)
    result += std::format("{:02x}", hash[i]);
  return result;
}

} // namespace dcpdoctor
