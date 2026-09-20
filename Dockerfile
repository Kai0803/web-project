FROM node:20-bookworm-slim

WORKDIR /app

COPY package*.json ./
RUN npm ci --omit=dev

COPY src ./src
COPY views ./views
COPY public ./public

ENV NODE_ENV=production
ENV PORT=3000
EXPOSE 3000

# SQLite 資料庫存放於 /app/data，請掛載 volume 以保留資料
VOLUME ["/app/data"]

CMD ["node", "src/app.js"]
