import { defineConfig } from "@playwright/test";
import base from "./playwright.config";
export default defineConfig({ ...base, testMatch: "extraction-processing.spec.ts", use: { ...base.use, video: "off", launchOptions: process.env.CLARIA_CHROMIUM ? { executablePath: process.env.CLARIA_CHROMIUM } : {} } });
