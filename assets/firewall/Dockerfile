# The firewall sidecar (SPEC 4.1 bis).
#
# It owns the network namespace its agent joins, and it is the only container
# in the pair with any capability. Everything it needs is here: a filtering
# resolver that answers for allowed names only, and a ruleset whose allowed
# addresses are fed by that resolver's own answers.
#
# Debian rather than Alpine for one measured reason: rule 4 needs dnsmasq's
# `nftset`, and Alpine's dnsmasq is built without it ("recompile with
# HAVE_NFTSET defined"). Debian's 2.90 has it.
FROM debian:bookworm-slim

RUN apt-get -qq update \
 && apt-get -qq install --no-install-recommends -y dnsmasq nftables bind9-dnsutils \
 && rm -rf /var/lib/apt/lists/*

COPY entrypoint.sh /usr/local/bin/entrypoint
COPY fw-ready /usr/local/bin/fw-ready
RUN chmod 0755 /usr/local/bin/entrypoint /usr/local/bin/fw-ready

# Exec form, no shell: the agent's life depends on this failing loudly.
ENTRYPOINT ["/usr/local/bin/entrypoint"]
