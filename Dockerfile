# --- Build stage ---
FROM node:20-alpine AS builder

WORKDIR /app

# Install server deps
COPY server/package*.json ./server/
RUN cd server && npm ci --production

# Install client deps and build
COPY client/package*.json ./client/
RUN cd client && npm ci
COPY client/ ./client/
RUN cd client && npm run build

# --- Production stage ---
FROM node:20-alpine

RUN apk add --no-cache tini
RUN npm install -g pm2

WORKDIR /app

# Copy server with deps
COPY --from=builder /app/server ./server
COPY --from=builder /app/client/dist ./client/dist

# Copy configs
COPY package.json ecosystem.config.js ./
COPY .env.production .env 2>/dev/null || true

# Create data/logs dirs
RUN mkdir -p server/data logs

EXPOSE 4500

# Use tini for proper signal handling
ENTRYPOINT ["/sbin/tini", "--"]

# Start with PM2 in production
CMD ["pm2-runtime", "ecosystem.config.js", "--env", "production"]
