# The prober (SPEC 4.1 bis, rule 7).
#
# `hq check` tries to get out of the perimeter from inside it. It does that
# from a container of its own rather than from the agent's, for one measured
# reason: the battery needs `nslookup`, `nc` and `wget`, and a stack image has
# no reason to carry them — the Rust image is Debian and has none, so every
# positive probe came back "refused" and `hq check` reported three violations
# that did not exist.
#
# Nothing is lost by moving them: the ruleset and the resolver belong to the
# **network namespace**, not to a container, and the prober joins the very
# namespace the agent joins. What it experiences is what the agent
# experiences — with tools that exist.
FROM alpine:3.22

RUN apk upgrade --no-cache
