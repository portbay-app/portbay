declare module "ansi-to-html" {
  interface ConvertOptions {
    fg?: string;
    bg?: string;
    newline?: boolean;
    escapeXML?: boolean;
    stream?: boolean;
    colors?: Record<number, string>;
  }

  export default class Convert {
    constructor(options?: ConvertOptions);
    toHtml(input: string): string;
  }
}
