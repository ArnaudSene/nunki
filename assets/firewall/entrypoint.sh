#!/bin/sh
# Pose the rules, then be the only resolver in the namespace.
#
# Two inputs, both written by nunki's Compose generator (SPEC 4.1 bis):
#   HQ_ALLOW_DOMAINS    comma-separated names the resolver may answer for
#   HQ_ALLOW_ADDRESSES  comma-separated addresses or CIDRs reachable by number
# Anything not named by one of the two is unreachable, and unresolvable.
set -eu

UPSTREAM="${HQ_UPSTREAM_DNS:-}"
DOMAINS="${HQ_ALLOW_DOMAINS:-}"
ADDRESSES="${HQ_ALLOW_ADDRESSES:-}"

rm -f /run/nunki/firewall.ready
mkdir -p /run/nunki

# The upstream resolver is the engine's, read before we take port 53 over.
if [ -z "$UPSTREAM" ]; then
    UPSTREAM=$(awk '/^nameserver/ { print $2; exit }' /etc/resolv.conf)
fi
[ -n "$UPSTREAM" ] || { echo "firewall: no upstream resolver found" >&2; exit 1; }

# dnsmasq runs as its own user so the ruleset can tell its queries — the only
# ones allowed to leave for the upstream resolver — from the agent's.
DNS_UID=$(id -u dnsmasq)
DNS_PORT=5353

conf=/run/nunki/dnsmasq.conf
{
    echo "no-hosts"
    echo "no-resolv"
    echo "bind-interfaces"
    echo "listen-address=127.0.0.1"
    # A high port, so the sidecar needs no capability to bind it: port 53 is
    # reached through the redirect below, which is what makes the tunnel
    # impossible to walk around anyway. Two capabilities, not three.
    echo "port=$DNS_PORT"
    echo "pid-file="
    echo "user=dnsmasq"
    # dnsmasq's built-in default group is `dip`, which exists on Debian and
    # nowhere else; naming it explicitly is what stops the drop from failing.
    echo "group=dnsmasq"
    # Everything not matched below is NXDOMAIN, answered here, never relayed:
    # this is what closes the DNS tunnel (rule 3).
    echo "local=/#/"
    for domain in $(echo "$DOMAINS" | tr ',' ' '); do
        [ -n "$domain" ] || continue
        echo "server=/$domain/$UPSTREAM"
        # Rule 4: an answer for an allowed name adds its address to the set
        # the ruleset consults, so a CDN that moves stays reachable without
        # any power living in the agent's container.
        echo "nftset=/$domain/4#inet#nunkifw#allowed4,6#inet#nunkifw#allowed6"
    done
} > "$conf"

nft -f - <<NFT
table inet nunkifw {
    set allowed4 { type ipv4_addr; flags timeout; timeout 1h; }
    set allowed6 { type ipv6_addr; flags timeout; timeout 1h; }
    set declared4 { type ipv4_addr; flags interval; }
    set declared6 { type ipv6_addr; flags interval; }

    chain output {
        type filter hook output priority filter; policy drop;
        ct state established,related accept
        oif lo accept
        meta skuid $DNS_UID udp dport 53 accept
        meta skuid $DNS_UID tcp dport 53 accept
        ip daddr @allowed4 accept
        ip6 daddr @allowed6 accept
        ip daddr @declared4 accept
        ip6 daddr @declared6 accept
        counter comment "refused by nunki"
    }

    chain input {
        type filter hook input priority filter; policy drop;
        ct state established,related accept
        iif lo accept
    }

    chain redirect_dns {
        type nat hook output priority -100; policy accept;
        meta skuid $DNS_UID return
        meta l4proto { tcp, udp } th dport 53 meta nfproto ipv4 dnat ip to 127.0.0.1:$DNS_PORT
        meta l4proto { tcp, udp } th dport 53 meta nfproto ipv6 dnat ip6 to [::1]:$DNS_PORT
    }
}
NFT

for entry in $(echo "$ADDRESSES" | tr ',' ' '); do
    [ -n "$entry" ] || continue
    case "$entry" in
        *:*) nft add element inet nunkifw declared6 "{ $entry }" ;;
        *)   nft add element inet nunkifw declared4 "{ $entry }" ;;
    esac
done

dnsmasq --conf-file="$conf" --keep-in-foreground &
dns=$!

# Ready means both halves are up. Until this file exists the agent does not
# start (rule 2).
tries=0
until nslookup -timeout=1 -type=a nunki-firewall-self-check.invalid 127.0.0.1 2>&1 | grep -q NXDOMAIN; do
    tries=$((tries + 1))
    [ "$tries" -lt 100 ] || { echo "firewall: the resolver never came up" >&2; kill $dns; exit 1; }
    sleep 0.1
done
touch /run/nunki/firewall.ready

wait $dns
