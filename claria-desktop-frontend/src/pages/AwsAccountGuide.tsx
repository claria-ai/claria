import StepIndicator from "../components/StepIndicator";
import type { Page } from "../App";

export default function AwsAccountGuide({
  navigate,
}: {
  navigate: (page: Page) => void;
}) {
  return (
    <div className="max-w-2xl mx-auto p-8">
      <StepIndicator current={1} />

      <h2 className="text-2xl font-bold mb-6">Step 1: Create an AWS Account</h2>

      <div className="space-y-4 text-gray-700">
        <p>
          Claria runs on your own AWS account. If you don't have one yet, follow
          these steps:
        </p>

        <ol className="list-decimal list-inside space-y-3 pl-2">
          <li>
            Go to{" "}
            <span className="font-mono text-sm bg-gray-100 px-1 py-0.5 rounded">
              aws.amazon.com
            </span>{" "}
            and click <strong>"Create an AWS Account"</strong>
          </li>
          <li>
            Enter your email address, choose a password, and follow the signup
            wizard
          </li>
          <li>
            <strong>Set up billing alerts</strong> to avoid surprise charges. In
            the AWS console, go to{" "}
            <span className="font-mono text-sm bg-gray-100 px-1 py-0.5 rounded">
              Billing &gt; Budgets
            </span>{" "}
            and create a budget (e.g. $10/month)
          </li>
          <li>
            <strong>Enable MFA</strong> on the root account (covered in the next
            step)
          </li>
        </ol>

        <div className="bg-blue-50 border border-blue-200 rounded-lg p-4 mt-6">
          <p className="text-blue-800 text-sm">
            Already have an AWS account? Skip ahead to the next step.
          </p>
        </div>
      </div>

      <div className="flex justify-between mt-8">
        <button
          onClick={() => navigate("start")}
          className="px-4 py-2 text-gray-600 hover:text-gray-800"
        >
          Back
        </button>
        <button
          onClick={() => navigate("guide-mfa")}
          className="px-6 py-2 bg-blue-500 text-white rounded-lg hover:bg-blue-600 transition-colors"
        >
          Next
        </button>
      </div>
    </div>
  );
}
