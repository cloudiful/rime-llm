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
