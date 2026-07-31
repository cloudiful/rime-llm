# Native Squirrel Plugin

The native plugin currently targets macOS Apple Silicon and the librime 1.16
ABI shipped by Squirrel. It is loaded from Squirrel's bundled
`Contents/Frameworks/rime-plugins` directory, not from the user Rime data
directory.

Build against the matching librime source checkout:

```bash
git clone --depth 1 --branch 1.16.0 https://github.com/rime/librime.git /tmp/librime-1.16.0
cmake -S native -B native/build \
  -DRIME_SOURCE_DIR=/tmp/librime-1.16.0 \
  -DRIME_LIBRARY="/Library/Input Methods/Squirrel.app/Contents/Frameworks/librime.1.dylib" \
  -DCMAKE_OSX_ARCHITECTURES=arm64
cmake --build native/build --parallel
ctest --test-dir native/build --output-on-failure
```

Install the resulting library into Squirrel's plugin directory, then redeploy
the Rime configuration:

```bash
sudo cp native/build/rime-plugins/librime-llm-predict.dylib \
  "/Library/Input Methods/Squirrel.app/Contents/Frameworks/rime-plugins/"
```

The package recipe installs `rime_ice_llm.schema.yaml` into the user's Rime
directory. It does not modify `default.custom.yaml`, schema selection, or any
other personal configuration. Squirrel updates may replace bundled plugins;
rebuild and reinstall the library after an app update if needed.

## Release artifact

Tagged GitHub releases include a macOS arm64 archive containing both the
`rime-llm` service and `librime-llm-predict.dylib`. The plugin is built against
Squirrel 1.1.2 and its librime 1.16.0 ABI. It is not built for Windows; the
Windows release contains only the CPU service executable.

After extracting the macOS archive, install the plugin with:

```bash
sudo cp librime-llm-predict.dylib \
  "/Library/Input Methods/Squirrel.app/Contents/Frameworks/rime-plugins/"
```

Then redeploy Rime. Create `config.toml` from the repository's
[`config.example.toml`](../config.example.toml) before starting the service
from the extracted directory. The model is downloaded on first start and is
intentionally not included in the release.

The release workflow verifies the Squirrel package checksum, librime ABI,
native test suite, target architecture, and SHA-256 checksums before publishing.
