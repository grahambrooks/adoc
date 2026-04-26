# Homebrew formula for the `adoc` AsciiDoc processor.
#
# Maintained by .github/workflows/release.yml — every successful
# release replaces this file with the version number and SHA256s of
# the freshly-published archives. Manual edits will be overwritten on
# the next release; if you need a custom installation, copy this file
# into a private tap.
#
# Install (after at least one release has been published):
#
#   brew tap grahambrooks/adoc https://github.com/grahambrooks/adoc
#   brew install adoc
#
# Or in one shot:
#
#   brew install grahambrooks/adoc/adoc
#
# Until the first release, `brew install` against this file fails
# with "no such tag" — the URLs below point at a release that hasn't
# been published yet. That's expected; trigger the Release workflow
# from GitHub Actions to publish the first version.
class Adoc < Formula
  desc "Rust command-line processor for the AsciiDoc Language specification"
  homepage "https://github.com/grahambrooks/adoc"
  version "0.0.0-pending"
  license "Apache-2.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/grahambrooks/adoc/releases/download/0.0.0-pending/adoc-0.0.0-pending-aarch64-apple-darwin.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    else
      url "https://github.com/grahambrooks/adoc/releases/download/0.0.0-pending/adoc-0.0.0-pending-x86_64-apple-darwin.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
  end

  on_linux do
    url "https://github.com/grahambrooks/adoc/releases/download/0.0.0-pending/adoc-0.0.0-pending-x86_64-unknown-linux-gnu.tar.gz"
    sha256 "0000000000000000000000000000000000000000000000000000000000000000"
  end

  def install
    bin.install "adoc"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/adoc --version")
  end
end
