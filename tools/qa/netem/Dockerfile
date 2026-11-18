# SPDX-License-Identifier: MPL-2.0
#
# QA-007 test-only image: an HTTP file server (nginx, Range requests like
# the core's stream proxy and media servers) whose outgoing traffic the
# test shapes with tc netem inside the container (`--cap-add NET_ADMIN`).
# Nothing from this image is linked into Kinolune.
FROM alpine:3.22
RUN apk add --no-cache iproute2-tc nginx
COPY nginx.conf /etc/nginx/nginx.conf
EXPOSE 8080
CMD ["nginx", "-g", "daemon off;"]
