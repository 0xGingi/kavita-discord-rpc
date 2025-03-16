FROM node:20-alpine

WORKDIR /app

COPY package.json package-lock.json* ./
RUN npm install --production

COPY server.js ./

RUN mkdir -p /app/images && \
    chmod 777 /app/images

VOLUME /app/images
EXPOSE 7589

CMD ["node", "server.js"] 