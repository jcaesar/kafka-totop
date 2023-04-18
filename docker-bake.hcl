group "default" {
  targets = ["linuxi", "linuxa64"]
}

group "arti" {
  targets = ["win32", "win64", "default"]
}

target "df" {
  dockerfile = "rel-cross.Dockerfile"
}

target "linux" {
  tags = ["docker.io/liftm/kafka-totop:latest"]
}

target "linuxi" {
  inherits = ["linux", "df"]
  platforms = ["linux/amd64"]
  args = {
    CARGO_BUILD_TARGET = "x86_64-unknown-linux-musl"
  }
}

target "linuxa64" {
  inherits = ["linux", "df"]
  platforms = ["linux/arm64"]
  args = {
    CARGO_BUILD_TARGET = "aarch64-unknown-linux-musl"
  }
}

# Doesn't work. :(
target "linuxa7" {
  inherits = ["linux", "df"]
  platforms = ["linux/arm/7"]
  args = {
    CARGO_BUILD_TARGET = "armv7-unknown-linux-musleabihf"
  }
}

target "win" {
  args = {
    EXE_SUFFIX = ".exe"
  }
}

target "win32" {
  inherits = ["win", "df"]
  tags = ["totop-win32"]
  args = {
    CARGO_BUILD_TARGET = "i686-pc-windows-gnu"
  }
}

target "win64" {
  inherits = ["win", "df"]
  tags = ["totop-win64"]
  args = {
    CARGO_BUILD_TARGET = "x86_64-pc-windows-gnu"
  }
}
