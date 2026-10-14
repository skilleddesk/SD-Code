FROM node:20-slim
WORKDIR /srv/app
COPY app/package.json ./
RUN npm install --omit=dev
COPY app/ ./
ENV PORT=8080
EXPOSE 8080
CMD ["node", "server.js"]
