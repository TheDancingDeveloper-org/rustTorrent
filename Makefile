all:

WEBUI_DIR ?= crates/librtbit/webui

@PHONY: webui-deps
webui-deps:
	cd desktop && npm install
	cd $(WEBUI_DIR) && npm install

@PHONY: webui-dev
webui-dev: webui-deps
	cd $(WEBUI_DIR) && \
	npm run dev

# NOTE: on LG TV using hostname is unstable for some reason, use IP address.
export RTBIT_UPNP_SERVER_ENABLE ?= true
export RTBIT_UPNP_SERVER_FRIENDLY_NAME ?= rtbit-dev
export RTBIT_HTTP_API_LISTEN_ADDR ?= [::]:3030
export RTBIT_ENABLE_PROMETHEUS_EXPORTER ?= true
export RTBIT_EXPERIMENTAL_UTP_LISTEN_ENABLE ?= true
export RTBIT_HTTP_API_ALLOW_CREATE ?= true
export RTBIT_FASTRESUME = true

CARGO_RUN_FLAGS ?=
RTBIT_OUTPUT_FOLDER ?= /tmp/scratch
RTBIT_POSTGRES_CONNECTION_STRING ?= postgres:///rtbit

# Alternatively run this on OSX to profile easily
# cargo instruments --profile release-debug --features=_disable_disk_write_net_benchmark -t time --time-limit 20000 -- download -o /tmp/scratch/ --overwrite
@PHONY: devserver-profile
devserver-profile:
	cargo run --release $(CARGO_RUN_FLAGS) -- server start $(RTBIT_OUTPUT_FOLDER)

# DEV variables (that's why defined after devserver-profile)
export RTBIT_LOG_FILE ?= /tmp/rtbit-log
export RTBIT_LOG_FILE_RUST_LOG ?= debug,librtbit=trace,upnp_serve=trace,librqbit_utp=debug
export CORS_ALLOW_REGEXP ?= '.*'

@PHONY: devserver
devserver:
	echo -n '' > $(RTBIT_LOG_FILE) && \
	cargo run $(CARGO_RUN_FLAGS) -- \
	server start $(RTBIT_OUTPUT_FOLDER)

@PHONY: devserver
devserver-postgres:
	echo -n '' > $(RTBIT_LOG_FILE) && \
	cargo run $(CARGO_RUN_FLAGS) -- \
	server start --fastresume --persistence-location $(RTBIT_POSTGRES_CONNECTION_STRING) $(RTBIT_OUTPUT_FOLDER)

@PHONY: check-repo
check-repo:
	bash scripts/check-repo-hygiene.sh

@PHONY: testserver
testserver:
	ulimit -n unlimited && cargo run -p librtbit --features http-api,tracing-subscriber-utils,webui,prometheus --example simulate_traffic

@PHONY: docker-build-one-platform
docker-build-one-platform:
	docker build \
		-t ikatson/rtbit:$(shell git describe --tags)-dev-$(shell echo $(PLATFORM) | tr '/' '-') \
		--platform $(PLATFORM) \
		-f Dockerfile .

@PHONY: docker-build-amd64
docker-build-amd64:
	PLATFORM=linux/amd64 $(MAKE) docker-build-one-platform

@PHONY: docker-build-armv7
docker-build-armv7:
		PLATFORM=linux/arm/v7 $(MAKE) docker-build-one-platform

@PHONY: clean
clean:
	rm -rf target

CARGO_RELEASE_PROFILE ?= release-github

@PHONY: release-linux-current-target
release-linux-current-target:
	CC_$(TARGET_SNAKE_CASE)=$(CROSS_COMPILE_PREFIX)-gcc \
	CXX_$(TARGET_SNAKE_CASE)=$(CROSS_COMPILE_PREFIX)-g++ \
	AR_$(TARGET_SNAKE_CASE)=$(CROSS_COMPILE_PREFIX)-ar \
	CARGO_TARGET_$(TARGET_SNAKE_UPPER_CASE)_LINKER=$(CROSS_COMPILE_PREFIX)-gcc \
	cargo build  --profile $(CARGO_RELEASE_PROFILE) --target=$(TARGET) --features=openssl-vendored,prometheus

@PHONY: debug-linux-docker-x86_64
debug-linux-docker-x86_64:
	docker build -t ikatson/rtbit:tmp-debug -f Dockerfile --platform linux/amd64 . && \
	docker push ikatson/rtbit:tmp-debug

@PHONY: release-linux-x86_64
release-linux-x86_64:
	TARGET=x86_64-unknown-linux-musl \
	TARGET_SNAKE_CASE=x86_64_unknown_linux_musl \
	TARGET_SNAKE_UPPER_CASE=X86_64_UNKNOWN_LINUX_MUSL \
	CROSS_COMPILE_PREFIX=x86_64-unknown-linux-musl \
	$(MAKE) release-linux-current-target

@PHONY: release-linux-aarch64
release-linux-aarch64:
	TARGET=aarch64-unknown-linux-musl \
	TARGET_SNAKE_CASE=aarch64_unknown_linux_musl \
	TARGET_SNAKE_UPPER_CASE=AARCH64_UNKNOWN_LINUX_MUSL \
	CROSS_COMPILE_PREFIX=aarch64-unknown-linux-musl \
	$(MAKE) release-linux-current-target

@PHONY: release-linux-armv7
release-linux-armv7:
	TARGET=armv7-unknown-linux-musleabihf \
	TARGET_SNAKE_CASE=armv7_unknown_linux_musleabihf \
	TARGET_SNAKE_UPPER_CASE=ARMV7_UNKNOWN_LINUX_MUSLEABIHF \
	CROSS_COMPILE_PREFIX=armv7-linux-musleabihf \
	$(MAKE) release-linux-current-target
