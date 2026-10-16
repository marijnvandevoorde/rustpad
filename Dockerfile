FROM docker.io/library/rust:1.95-alpine AS backend
WORKDIR /home/rust/src
RUN apk --no-cache add musl-dev openssl-dev
COPY . .
# Tests run in the debug profile, so the release build is the only LTO build.
RUN cargo test
RUN cargo build --release

# The WebAssembly and frontend stages produce platform-independent files, so
# they run on the build machine's own platform.
FROM --platform=$BUILDPLATFORM docker.io/library/rust:1.95-alpine AS wasm
ARG WASM_PACK_VERSION=0.15.0
WORKDIR /home/rust/src
RUN apk --no-cache add curl musl-dev \
    && rustup target add wasm32-unknown-unknown \
    && name="wasm-pack-v${WASM_PACK_VERSION}-$(uname -m)-unknown-linux-musl" \
    && curl -sSfL "https://github.com/rustwasm/wasm-pack/releases/download/v${WASM_PACK_VERSION}/${name}.tar.gz" \
    | tar -xz -C /usr/local/bin --strip-components=1 "${name}/wasm-pack"
COPY . .
RUN wasm-pack build rustpad-wasm

FROM --platform=$BUILDPLATFORM docker.io/library/node:24-alpine AS frontend
WORKDIR /usr/src/app
COPY package.json package-lock.json ./
COPY --from=wasm /home/rust/src/rustpad-wasm/pkg rustpad-wasm/pkg
RUN npm ci
COPY . .
ARG GITHUB_SHA
ENV VITE_SHA=${GITHUB_SHA}
RUN npm run check
RUN npm run build

FROM scratch
COPY --from=frontend /usr/src/app/dist dist
COPY --from=backend /home/rust/src/target/release/rustpad-server .
USER 1000:1000
CMD [ "./rustpad-server" ]
