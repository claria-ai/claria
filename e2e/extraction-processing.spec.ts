import { expect, test } from "@playwright/test";
import { buildInitScript } from "./tauri-mock.js";
const url = process.env.CLARIA_TEST_URL ?? "http://localhost:1420";
test.setTimeout(30000);
const output = "../screenshots/review/extraction-processing";
test.beforeEach(async ({ page }) => {
  await page.addInitScript({ content: buildInitScript({ configured: true }) + `(() => {
    const original = window.__TAURI_INTERNALS__.invoke;
    const listeners = {};
    let configured = false;
    const rows = [{filename:"assessment.pdf",size:12800,uploaded_at:null,processing_status:{status:"unknown"}}];
    window.__dropExtractionFiles = (paths) => { for (const callback of listeners["tauri://drag-drop"] || []) window["_" + callback]({event:"tauri://drag-drop",payload:{paths,position:{x:200,y:200}}}); };
    window.__TAURI_INTERNALS__.invoke = async (cmd,args) => {
      if(cmd === "plugin:event|listen") (listeners[args.event] ||= []).push(args.handler);
      if(cmd === "load_config" || cmd === "fetch_cloud_preferences") return {...await original("load_config",{}),model_tuning:{reasoning_enabled:true,effort:null,temperature:null},draft_pipeline:{},chat_streaming:"paragraph",extraction_model_id:configured ? "us.anthropic.claude-sonnet-4-20250514-v1:0":null};
      if(cmd === "save_preferences_patch") {configured=!!args.patch.extraction_model_id; return {...await original("load_config",{}),model_tuning:{reasoning_enabled:true,effort:null,temperature:null},draft_pipeline:{},chat_streaming:"paragraph",extraction_model_id:args.patch.extraction_model_id};}
      if(cmd === "list_record_files") return rows;
      if(cmd === "list_chat_histories" || cmd === "list_report_sessions") return [];
      if(cmd === "upload_record_file") {const file={filename:args.filePath.split('/').pop(),size:12800,uploaded_at:null,processing_status:{status:"failed",code:"throttled",reason:"AWS limited this processing request. Wait briefly, then retry."}}; rows.push(file); return file;}
      if(cmd === "extract_record_file") {const file=rows.find(file=>file.filename===args.filename);if(file)file.processing_status={status:"ready"};return {filename:args.filename,text:"Synthetic extracted assessment."};}
      return original(cmd,args);
    };
  })();` });
});
test("independent model setting and unconfigured state", async ({page}) => {
  await page.goto(url);
  await page.getByRole("button",{name:"Preferences",exact:true}).click();
  await page.getByText("Document Extraction",{exact:true}).click();
  await page.getByLabel("Document extraction model",{exact:true}).scrollIntoViewIfNeeded();
  await page.screenshot({path:`${output}/extraction-model-unconfigured.png`});
  await page.getByLabel("Document extraction model",{exact:true}).selectOption({index:1});
  await expect(page.getByLabel("Document extraction model",{exact:true})).toBeEnabled();
  await page.screenshot({path:`${output}/extraction-model-settings.png`});
});
test("historical unknown, batch failures, partial retry and completion",async ({page}) => {
  await page.goto(url);
  await page.getByRole("button",{name:"Client Files"}).click();
  await page.getByText("Jane Doe",{exact:true}).click();
  await expect(page.getByText("No extracted text")).toBeVisible();
  await page.screenshot({path:`${output}/extraction-historical-missing-text.png`});
  await page.evaluate(() => (window as any).__dropExtractionFiles(["/synthetic/intake.pdf","/synthetic/referral.docx"]));
  await expect(page.getByText("2 files failed to process. Review the reasons below.")).toBeVisible();
  await page.screenshot({path:`${output}/extraction-batch-failures.png`});
  await page.getByRole("dialog").getByRole("button",{name:"Retry processing",exact:true}).first().click();
  await expect(page.getByText("1 file failed to process. Review the reasons below.")).toBeVisible();
  await expect(page.getByRole("button",{name:"Done",exact:true})).toBeFocused();
  await page.screenshot({path:`${output}/extraction-batch-partial-success.png`});
  await page.getByRole("dialog").getByRole("button",{name:"Retry processing",exact:true}).click();
  await expect(page.getByRole("heading",{name:"Text processing complete"})).toBeVisible();
  await page.screenshot({path:`${output}/extraction-retry-complete.png`});
  await page.getByRole("button",{name:"Done",exact:true}).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
});
