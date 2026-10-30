{
  description = "Rust devShell with a pinned rust-overlay toolchain";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { nixpkgs, rust-overlay, ... }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-darwin"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      devShells = forAllSystems (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ rust-overlay.overlays.default ];
          };
          # 需要严格钉版本时用：rust-bin.stable."1.98.1".default
          # nightly 用：rust-bin.selectLatestNightlyWith (t: t.default.override { ... })
          mkRust =
            targets:
            pkgs.rust-bin.stable.latest.default.override {
              extensions = [
                "rust-analyzer"
                "rust-src"
              ];
              inherit targets;
            };
          mkDevShell =
            rust: extra:
            pkgs.mkShell (
              {
                # 项目工具按需加在这里，例如 pkgs.cargo-nextest / pkgs.sqlx-cli；
                # 交叉编译再加 pkgs.cargo-zigbuild + pkgs.zig_0_15。
                # mold 仅 Linux；供 .cargo/config.toml 的可选链接加速使用。
                packages = [
                  rust
                  pkgs.cargo-nextest
                ];
                RUST_SRC_PATH = "${rust}/lib/rustlib/src/rust/library";

                # 全部产物集中存放；真实 workspace 路径区分 worktree，子 crate 共用。
                # 每次进入都重算，覆盖从其他 devShell 或旧全局配置继承的目录。
                shellHook = ''
                  if ! rust_workspace_manifest="$(${rust}/bin/cargo locate-project --workspace --message-format plain)" \
                    || ! rust_workspace_manifest="$(${pkgs.coreutils}/bin/realpath -- "$rust_workspace_manifest")" \
                    || ! rust_workspace_hash="$(printf '%s' "$rust_workspace_manifest" | ${pkgs.coreutils}/bin/sha256sum)"; then
                    echo "Rust devShell: 请从目标 Cargo workspace 内进入环境。" >&2
                    exit 1
                  fi
                  export CARGO_TARGET_DIR="$HOME/.cargo/target/workspaces/''${rust_workspace_hash%% *}"
                  export CARGO_BUILD_BUILD_DIR="$CARGO_TARGET_DIR"
                  unset rust_workspace_manifest rust_workspace_hash
                '';
              }
              // extra
            );
        in
        {
          default = mkDevShell (mkRust [ ]) { };
        }
        // pkgs.lib.optionalAttrs (system == "x86_64-linux") {
          # Run real 32-bit binaries on a Linux host with IA32 support. The
          # pinned compiler wrapper supplies the matching glibc and ELF loader.
          index32 = mkDevShell (mkRust [ "i686-unknown-linux-gnu" ]) {
            CARGO_TARGET_I686_UNKNOWN_LINUX_GNU_LINKER = "${pkgs.pkgsi686Linux.stdenv.cc}/bin/cc";
          };
        }
      );
    };
}
