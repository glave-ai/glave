# Minimal TypeScript eval base. node + tsx (TS execution) + node's built-in
# test runner. Interpreted, so the runtime stays small.
FROM node:alpine

RUN apk add --no-cache ripgrep
RUN npm install -g tsx
WORKDIR /work
CMD ["tail", "-f", "/dev/null"]
