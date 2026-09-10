# The firewall sidecar (SPEC 4.1 bis).
#
# It owns the network namespace its agent joins, and it is the only container
# in the pair with any capability. Everything it needs is here: a filtering
# resolver that answers for allowed names only, and a ruleset whose allowed
# addresses are fed by that resolver's own answers.
#
# Alpine, and dnsmasq built rather than installed. Measured on 2026-09-09:
# `debian:bookworm-slim` carries 4 critical and 52 high vulnerabilities, none
# of them fixable — Debian marks them won't-fix — and the finished sidecar
# came to 8 critical, 55 high, 154 MB. On Alpine the same sidecar is 0 and 0,
# at 27 MB. That matters here more than elsewhere: the agent shares this
# container's network namespace, so this resolver's listening socket is
# reachable by the very process the sidecar exists to fence in.
#
# Alpine's packaged dnsmasq is built without `nftset` ("recompile with
# HAVE_NFTSET defined"), and `nftset` is what rule 4 needs — the resolver
# feeding the ruleset the addresses it just handed out. So we build it, which
# costs one stage and buys the whole security posture.

FROM alpine:3.22 AS build

RUN apk add --no-cache build-base linux-headers nftables-dev

ARG DNSMASQ_VERSION=2.91
# Trust on first use: this is the digest of what upstream served on
# 2026-09-09, pinned so that a later change is a build failure rather than a
# silent substitution. Verifying Simon Kelley's signature would be stronger,
# and is not done here.
ARG DNSMASQ_SHA256=f622682848b33677adb2b6ad08264618a2ae0a01da486a93fd8cd91186b3d153

RUN wget -qO dnsmasq.tar.xz "https://thekelleys.org.uk/dnsmasq/dnsmasq-${DNSMASQ_VERSION}.tar.xz" \
 && echo "${DNSMASQ_SHA256}  dnsmasq.tar.xz" | sha256sum -c - \
 && tar xf dnsmasq.tar.xz \
 && cd "dnsmasq-${DNSMASQ_VERSION}" \
 && make COPTS="-DHAVE_NFTSET" -j"$(nproc)" \
 && install -m 0755 src/dnsmasq /dnsmasq

FROM alpine:3.22

# `upgrade` before `add`: the base image lags its own repository, and the two
# openssl advisories it ships have fixes waiting.
RUN apk upgrade --no-cache \
 && apk add --no-cache nftables bind-tools \
 && addgroup -S dnsmasq \
 && adduser -S -G dnsmasq -H -s /sbin/nologin dnsmasq

COPY --from=build /dnsmasq /usr/sbin/dnsmasq
COPY entrypoint.sh /usr/local/bin/entrypoint
COPY fw-ready /usr/local/bin/fw-ready
RUN chmod 0755 /usr/local/bin/entrypoint /usr/local/bin/fw-ready

# Exec form, no shell: the agent's life depends on this failing loudly.
ENTRYPOINT ["/usr/local/bin/entrypoint"]
