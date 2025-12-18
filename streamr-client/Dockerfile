FROM node:20-alpine

# Create app dir
WORKDIR /usr/src/app

# Install build deps (git for private deps) and runtime deps
RUN apk add --no-cache cmake make g++ python3  py3-setuptools openssl-dev

# Copy package files first to leverage layer caching
COPY package*.json ./

# Install dependencies
RUN npm install --only=production

# Copy app sources
COPY . .

# Optional: expose port if the client serves HTTP (adjust if different)
EXPOSE 3000

# Use non-root user provided by the image
USER node

# Default command — adjust if your start script differs (e.g. "npm start")
CMD ["node", "index.js"]