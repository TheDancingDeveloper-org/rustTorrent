FROM alpine:3.23
RUN apk add --no-cache transmission-daemon curl
RUN mkdir -p /config /data/testdata /data/torrents
COPY settings.json /config/settings.json
EXPOSE 9091 51413
HEALTHCHECK --interval=3s --timeout=2s --retries=10 \
    CMD curl -so /dev/null -w '%{http_code}' http://localhost:9091/transmission/rpc | grep -qE '(200|409)'
CMD ["transmission-daemon", "--foreground", "--config-dir", "/config", "--no-portmap", "--log-level=error"]
