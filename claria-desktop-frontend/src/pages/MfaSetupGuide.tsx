import StepIndicator from "../components/StepIndicator";
import type { Page } from "../App";

export default function MfaSetupGuide({
  navigate,
}: {
  navigate: (page: Page) => void;
}) {
  return (
    <div className="max-w-2xl mx-auto p-8">
      <StepIndicator current={2} />

      <h2 className="text-2xl font-bold mb-6">
        Step 2: Secure Your Root Account with MFA
      </h2>

      <div className="space-y-4 text-gray-700">
        <div className="bg-red-50 border border-red-200 rounded-lg p-4">
          <p className="text-red-800 text-sm font-medium">
            Highly recommended. Your root account has unrestricted access to
            everything in your AWS account. Without MFA, anyone who obtains your
            password can take full control — including deleting data, running up
            charges, and accessing client records.
          </p>
        </div>

        <p>
          AWS supports passkeys (biometrics or device PIN) and hardware security
          keys as the strongest MFA options. Follow these steps to enable one:
        </p>

        <ol className="list-decimal list-inside space-y-3 pl-2">
          <li>
            Sign in to the{" "}
            <span className="font-mono text-sm bg-gray-100 px-1 py-0.5 rounded">
              AWS Management Console
            </span>{" "}
            as the <strong>root user</strong> (the email you signed up with)
          </li>
          <li>
            Click your account name in the top-right corner and select{" "}
            <strong>Security credentials</strong>
          </li>
          <li>
            Scroll to the <strong>Multi-factor authentication (MFA)</strong>{" "}
            section and click <strong>"Assign MFA device"</strong>
          </li>
          <li>
            Enter a device name (e.g.{" "}
            <span className="font-mono text-sm bg-gray-100 px-1 py-0.5 rounded">
              my-passkey
            </span>
            ), select <strong>"Passkey or Security Key"</strong>, and click{" "}
            <strong>Next</strong>
          </li>
          <li>
            Your browser will prompt you to create a passkey — choose one of:
            <ul className="list-disc list-inside pl-6 mt-2 space-y-1 text-sm">
              <li>
                <strong>Biometric</strong> (fingerprint or face recognition on
                your device)
              </li>
              <li>
                <strong>Device PIN</strong> (your computer's lock-screen PIN)
              </li>
              <li>
                <strong>Security key</strong> (insert a FIDO2 key into USB and
                tap it)
              </li>
            </ul>
          </li>
          <li>
            Follow the browser prompts to select where to store the passkey,
            then click <strong>Continue</strong>
          </li>
        </ol>

        <div className="bg-blue-50 border border-blue-200 rounded-lg p-4 mt-6">
          <p className="text-blue-800 text-sm">
            Once set up, AWS will require this passkey every time you sign in as
            the root user. You can add additional MFA devices later from the same
            Security credentials page.
          </p>
        </div>
      </div>

      <div className="flex justify-between mt-8">
        <button
          onClick={() => navigate("guide-aws")}
          className="px-4 py-2 text-gray-600 hover:text-gray-800"
        >
          Back
        </button>
        <div className="flex gap-3">
          <button
            onClick={() => navigate("guide-access-key")}
            className="px-4 py-2 text-amber-700 hover:text-amber-900 text-sm"
          >
            Skip (not recommended)
          </button>
          <button
            onClick={() => navigate("guide-access-key")}
            className="px-6 py-2 bg-blue-500 text-white rounded-lg hover:bg-blue-600 transition-colors"
          >
            Done — Next
          </button>
        </div>
      </div>
    </div>
  );
}
