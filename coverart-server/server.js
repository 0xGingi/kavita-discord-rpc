import express from 'express';
import fetch from 'node-fetch';
import cors from 'cors';
import crypto from 'crypto';
import fs from 'fs/promises';
import path from 'path';
import { fileURLToPath } from 'url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const app = express();
const PORT = process.env.PORT || 7589;
const IMAGE_DIR = path.join(__dirname, 'images');

try {
  await fs.mkdir(IMAGE_DIR, { recursive: true });
  console.log(`Created images directory: ${IMAGE_DIR}`);
} catch (err) {
  console.error(`Error creating images directory: ${err}`);
}

app.use(cors());
app.use(express.json());
app.use(express.raw({ type: 'image/*', limit: '10mb' }));

app.get('/proxy', async (req, res) => {
  try {
    const imageUrl = req.query.url;
    
    if (!imageUrl) {
      return res.status(400).send('Missing URL parameter');
    }
    
    const response = await fetch(imageUrl);
    
    if (!response.ok) {
      return res.status(response.status).send('Failed to fetch image');
    }
    
    const contentType = response.headers.get('content-type');
    
    res.setHeader('Content-Type', contentType);
    res.setHeader('Cache-Control', 'public, max-age=86400');
    
    response.body.pipe(res);
  } catch (error) {
    console.error('Error proxying image:', error);
    res.status(500).send('Server error');
  }
});

app.post('/upload', async (req, res) => {
  console.log('Received upload request');
  console.log('Content type:', req.headers['content-type']);
  console.log('Request body size:', req.body?.length || 0);
  
  try {
    if (!req.body || req.body.length === 0) {
      console.log('No image data received');
      return res.status(400).send('No image data provided');
    }
    
    const hash = crypto.createHash('md5').update(req.body).digest('hex');
    const imageId = `${hash}-${Date.now()}`;
    const contentType = req.headers['content-type'] || 'image/jpeg';
    const ext = contentType.split('/')[1] || 'jpg';
    const filename = `${imageId}.${ext}`;
    const filePath = path.join(IMAGE_DIR, filename);
    
    await fs.writeFile(filePath, req.body);
    console.log(`Saved image to ${filePath}`);
    
    const imageUrl = `/images/${filename}`;
    console.log(`Returning URL: ${imageUrl}`);
    res.json({ url: imageUrl });
    
    cleanupOldImages();
    
  } catch (error) {
    console.error('Error uploading image:', error);
    res.status(500).send('Server error');
  }
});

app.use('/images', express.static(IMAGE_DIR));

app.all('*', (req, res) => {
  console.log(`Received request for non-existent path: ${req.method} ${req.path}`);
  res.status(404).send(`Path not found: ${req.path}`);
});

async function cleanupOldImages() {
  try {
    const files = await fs.readdir(IMAGE_DIR);
    const now = Date.now();
    const oneDayMs = 24 * 60 * 60 * 1000;
    
    for (const file of files) {
      try {
        const filePath = path.join(IMAGE_DIR, file);
        const stats = await fs.stat(filePath);
        const fileAge = now - stats.mtime.getTime();
        
        if (fileAge > oneDayMs) {
          await fs.unlink(filePath);
          console.log(`Deleted old image: ${file}`);
        }
      } catch (err) {
        console.error(`Error checking file ${file}:`, err);
      }
    }
  } catch (err) {
    console.error('Error cleaning up old images:', err);
  }
}

app.listen(PORT, () => {
  console.log(`Image proxy server running on port ${PORT}`);
}); 