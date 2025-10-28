# Information Blocking Analysis: Trade Secret Licensing Requirements for EHR APIs

## Executive Summary

**The Case:** An EHR vendor requiring trade secret licensing (with non-disclosure restrictions) for API access likely constitutes information blocking, even though the licensing exception (45 CFR § 171.303) explicitly allows NDAs under certain conditions.

**Why:** The Conditions of Certification for certified APIs (45 CFR § 170.404) require that API documentation be publicly accessible "without any preconditions or additional steps" - a requirement that appears to supersede or limit the licensing exception's NDA provisions for certified API technology specifically.

**Strength of Case:** Moderate to Strong for certified APIs, with some nuance required.

---

## Legal Framework

### 1. Information Blocking Definition (45 CFR Part 171)

Information blocking is defined as:
> "A practice by an actor that is **likely to interfere with** the access, exchange, or use of electronic health information (EHI), except as required by law or specified in an information blocking exception."

**Key Standard:** For health IT developers, the standard is whether they "know, or should know, that a practice is likely to interfere" with access, exchange, or use of EHI.

**Critical Point:** The practice need not actually prevent access - it only needs to be "likely to interfere with, prevent, or materially discourage" access, exchange, or use.

### 2. API Certification Requirements (45 CFR § 170.404)

For **certified API technology**, developers must:

> "Publish complete business and technical documentation via a **publicly accessible hyperlink that allows any person to directly access the information without any preconditions or additional steps**."

This includes:
- All technical requirements and configurations for the API
- Terms and conditions for use
- Authentication and registration requirements
- All fee information

**Prohibited Practices:**
- Charging any fee for access to required API documentation
- Charging for "other types of documentation or information that a software developer may reasonably require to make effective use of certified API technology for any legally permissible purpose"

**Effective Date:** April 5, 2021

### 3. Licensing Exception (45 CFR § 171.303)

The licensing exception allows actors to license "interoperability elements" for EHI access, but with strict conditions:

**Regarding NDAs Specifically (§ 171.303(b)(5)):**

The actor may require a non-disclosure agreement that is:
1. "**No broader than necessary** to prevent unauthorized disclosure of the actor's trade secrets"
2. Must "**state with particularity** all information the actor claims as trade secrets"
3. Such information must meet the definition of a trade secret under applicable law

**Other Key Restrictions:**
- Must negotiate in good faith within specific timelines (10 days to start, 30 days to complete)
- Terms must be reasonable and non-discriminatory
- Cannot require non-competition, exclusive dealing, or unbundling of unrelated products
- Royalties must be based on independent value of the technology, not strategic control value

---

## The Case for Information Blocking

### Argument 1: Direct Violation of § 170.404 for Certified APIs

**The Strongest Argument**

If the EHR has **certified API technology** (which most modern EHRs do under the Cures Act), requiring an NDA to access API documentation directly violates § 170.404(a)(2)(i).

**Analysis:**
- The regulation explicitly requires documentation be accessible "**without any preconditions or additional steps**"
- An NDA is definitionally a precondition to access
- The phrase "any person" suggests the broadest possible access, incompatible with requiring legal agreements
- The regulation specifically prohibits charging for documentation that developers "may reasonably require to make effective use of certified API technology"

**Why This Matters for Open Source:**
- Open source development requires the ability to publicly share implementation details, discuss technical approaches, and collaborate openly
- An NDA requiring non-disclosure of "how the API works" is fundamentally incompatible with open source development
- This creates a de facto exclusion of open source developers from the ecosystem

**Counterargument Consideration:**
Could a vendor argue that § 171.303 (licensing exception) allows the NDA? **Unlikely to succeed** because:
1. § 170.404 is a Condition of Certification - a mandatory requirement for certified health IT
2. The search results confirm that § 170.404 and § 171.303 work together, with § 170.404 setting specific mandatory requirements for certified APIs
3. One ONC source states: "If the actor is a health IT developer subject to the Conditions of Certification in § 170.404, the actor must comply with all requirements of such conditions for all practices and at all relevant times"

### Argument 2: NDAs That Prevent Open Source Development Are "Broader Than Necessary"

**A Complementary Argument Under § 171.303 Itself**

Even if we assume the licensing exception applies, an NDA that effectively prevents open source development likely fails the § 171.303(b)(5) requirements.

**Analysis:**

The regulation requires NDAs be "**no broader than necessary** to prevent unauthorized disclosure of the actor's trade secrets."

**What might legitimately be trade secrets?**
- Proprietary algorithms or business logic unique to the vendor
- Security mechanisms or vulnerability information
- Specific implementation details of proprietary features

**What is NOT protectable as trade secrets in this context?**
- The API's interface specification (methods, endpoints, parameters)
- How to call the API and what responses to expect
- Standard FHIR resources and their mappings
- Technical requirements for authentication and integration

**Why an NDA preventing discussion of API functionality is "broader than necessary":**

1. **The "stated with particularity" requirement:** The NDA must specify exactly what is claimed as a trade secret. A blanket prohibition on discussing "how the API works" fails this requirement.

2. **Incompatibility with standardized APIs:** Certified APIs must use HL7 FHIR Release 4.0.1 as the foundational standard. The standard itself is public. An NDA can't reasonably claim that using a public standard in a documented way is a trade secret.

3. **The non-discrimination requirement:** If the vendor allows proprietary (closed-source) developers to build applications but effectively excludes open source developers through NDA restrictions, this violates the requirement that terms be "based on objective and verifiable criteria that are uniformly applied for all similarly situated classes of persons."

### Argument 3: "Likely to Interfere" Standard and Open Source Exclusion

**A Policy-Based Argument**

The ONC has made clear that the Cures Act aims to "promote an entrepreneurial economy" and "inject competition into health care" through an open developer ecosystem.

**Analysis:**

Preventing open source development is "likely to interfere with" EHI access, exchange, and use because:

1. **Material discouragement:** Open source developers who cannot publicly document their integration work, seek community help, or share code are materially discouraged from developing applications.

2. **Reduction in competition:** ONC specifically prohibited "anticompetitive behaviors" in the final rule. Excluding open source developers reduces competition and innovation.

3. **Ecosystem effects:** Open source projects often serve as:
   - Reference implementations that benefit the entire ecosystem
   - Training resources for developers
   - Low-cost or free solutions for smaller healthcare organizations
   - Foundations for innovation

4. **Developer knowledge standard:** Health IT developers "know, or should know" that NDA requirements incompatible with open source development would exclude a significant class of developers and reduce competition.

---

## Potential Weaknesses and Counterarguments

### Weakness 1: "The NDA doesn't explicitly forbid open source"

**Counterargument:** True, but:
- An NDA requiring non-disclosure of API functionality is functionally incompatible with open source development
- The regulations look to effects, not just explicit prohibitions
- The "likely to interfere" standard captures practices that materially discourage access, even if they don't absolutely prevent it

### Weakness 2: "The licensing exception explicitly allows NDAs"

**Counterargument:**
- Only for information that meets the trade secret definition and is stated with particularity
- Only to the extent "no broader than necessary"
- Must be reconciled with § 170.404's "no preconditions" requirement for certified APIs
- The two provisions work together, with § 170.404 setting mandatory requirements for certified API documentation

### Weakness 3: "We're only protecting our proprietary implementation, not the API specification"

**Response:** This could be a valid defense **if**:
- The NDA is narrowly tailored to specific proprietary elements clearly identified as trade secrets
- Standard API documentation (how to call endpoints, expected responses, data models) is publicly available without NDA
- Open source developers can build fully functional applications without accessing NDA-protected information

But if developers need NDA-protected information to build working applications, this defense fails.

### Weakness 4: Distinguishing Documentation from Actual Access

**A Nuance Worth Considering:**

There's a potential distinction between:
- **API Documentation** (must be publicly accessible without preconditions under § 170.404)
- **API Access** (the actual ability to connect and exchange data)

A vendor might argue: "We publish the documentation publicly, but you need a license/NDA to actually connect your application to production systems."

**Assessment of this argument:**
- **For development purposes:** Weak. The prohibition on fees includes fees for documentation developers "may reasonably require to make effective use of certified API technology." If an NDA is required for effective use, it's likely prohibited.
- **For production deployment:** Stronger. It may be reasonable to require licensing for production access to live patient data, including reasonable security and privacy terms.
- **Critical distinction:** Reasonable licensing for production access ≠ NDAs that prevent discussing or sharing how the API works

---

## The Open Source Incompatibility Problem

### Why NDAs Fundamentally Conflict with Open Source Development

**Open source development requires:**
1. Public documentation of integration approaches
2. Ability to share code that demonstrates API usage
3. Community discussion of implementation challenges and solutions
4. Public bug reports and feature requests
5. Transparent development processes

**Trade secret NDAs typically prohibit:**
1. Disclosing "confidential information" about the API
2. Sharing implementation details
3. Public discussion of API behavior
4. Documentation of undocumented features or behaviors

**The incompatibility:** Even if an NDA says "you can do open source," if it prohibits disclosing how the API works, it effectively prevents meaningful open source development of integrations.

### Real-World Impact

If a developer under NDA cannot:
- Publish code comments explaining why certain API calls are made in a specific way
- Discuss on public forums why the API behaves differently than documented
- Share test cases that demonstrate API behavior
- Contribute to community knowledge about integration best practices

Then open source development is effectively prohibited, even if not explicitly forbidden.

---

## Recommendations and Conclusions

### Strength of the Case

**For Certified API Technology:** **Strong**

The combination of:
1. § 170.404's explicit "no preconditions" requirement
2. The prohibition on fees for documentation needed for effective use
3. ONC's clear policy intent to promote open competition
4. The "likely to interfere" standard that captures material discouragement

Creates a compelling case that requiring NDAs for certified API access violates information blocking rules.

### For Non-Certified Interoperability Elements: **Moderate**

The case is weaker but still viable, relying primarily on:
1. The "no broader than necessary" and "stated with particularity" requirements of § 171.303(b)(5)
2. The "likely to interfere" standard
3. Non-discrimination requirements

### The Nuanced Middle Ground

A vendor could potentially:
1. ✅ Publish all API documentation publicly without preconditions (required)
2. ✅ Allow open source development based on public documentation (required)
3. ✅ Require reasonable registration for production API access (likely permissible)
4. ✅ Require NDAs for truly proprietary implementation details not needed for basic integration (potentially permissible if narrow and specific)
5. ❌ Require NDAs that prevent discussing how the API works (prohibited)
6. ❌ Require NDAs as a precondition to accessing API documentation (prohibited for certified APIs)

### Your Concern is Well-Founded

You are correct that:
1. Licensing for production connectivity may be reasonable
2. But preventing open source development violates the spirit and likely the letter of information blocking rules
3. The case is strongest for certified APIs under § 170.404
4. Even under the licensing exception, NDAs must be narrowly tailored and cannot be broader than necessary

The regulations were explicitly designed to "promote an entrepreneurial economy" and prevent "anticompetitive behaviors." Excluding open source developers through trade secret restrictions that prevent public development is antithetical to these goals and likely violates the information blocking provisions.

---

## Sources

- 45 CFR Part 171 - Information Blocking
- 45 CFR § 170.404 - Application Programming Interfaces (Conditions of Certification)
- 45 CFR § 171.303 - Licensing Exception
- ONC 21st Century Cures Act Final Rule (Federal Register, May 1, 2020)
- ONC HTI-1 Final Rule updates
- HealthIT.gov resources and guidance documents
