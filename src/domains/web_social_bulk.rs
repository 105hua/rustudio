//! BulkPurchaseApp result boundary. Kept private until the complete prompt,
//! browser upsell and native completion transport are implemented together.
//! Source: useBulkPurchaseRequestListenerEffect, getItemResultsFromResponse,
//! lineItemToStatus, getCommonItemStatus, selectResultItems/selectRobuxSpent.
#[allow(dead_code)]
const SOURCE: &str = r#"
local ctx=...;local module={}
local function enum(t,n)return ctx.enum and ctx.enum(t,n)or Enum[t][n]end
local reasons={INSUFFICIENT_ROBUX='InsufficientRobux',QUOTA_EXCEEDED='QuotaExceeded',NOT_FOR_SALE='NotForSale',NOT_AVAILABLE_FOR_PURCHASER='NotAvailableForPurchaser',PRICE_MISMATCH='PriceMismatch',SOLD_OUT='SoldOut',PURCHASER_IS_SELLER='PurchaserIsSeller',QUANTITY_LIMIT_EXCEEDED='QuantityLimitExceeded',ALREADY_OWNED='AlreadyOwned',INSUFFICIENT_MEMBERSHIP='InsufficientMembership',PURCHASE_PLACE_INVALID='PlaceInvalid'}
function module.normalise(items)
    local out={}
    for _,item in items do
        local copy={}
        for _,key in {'id','name','description','priceRobux','type','ownedCount','quantityLimitPerUser','isLimited','discountInformation','purchaseOptions'}do copy[key]=item[key]end
        copy.purchasableState=item.purchaseableState=='Purchaseable'and'Purchasable'or item.purchaseableState
        table.insert(out,copy)
    end
    return out
end
function module.line_status(line)
    -- SUCCEEDED wins even if an errorReason accompanies it.
    return enum('MarketplaceItemPurchaseStatus',line.status=='SUCCEEDED'and'Success'or reasons[line.errorReason]or'SystemError')
end
function module.statuses(response,items)
    local groups=response.fulfillmentGroups
    local lines=groups and groups[1]and groups[1].lineItems or{}
    local out={};local index=0
    for _,item in items do
        if item.purchasableState=='Purchasable'then
            index+=1
            if lines[index]then out[item.id]=module.line_status(lines[index])end
        end
    end
    return out
end
function module.common(statuses)
    local common
    for _,status in statuses do
        if common~=nil and common~=status then return nil end
        if common==nil then common=status end
    end
    return common
end
function module.view(statuses)
    local common=module.common(statuses)
    if common==nil then return'PartialSuccess'end
    if common==enum('MarketplaceItemPurchaseStatus','Success')then return'Success'end
    return'Error'
end
function module.results(items,statuses)
    local out={RobuxSpent=0,Items={}}
    for _,item in items do
        local status=statuses[item.id]or enum('MarketplaceItemPurchaseStatus','SystemError')
        table.insert(out.Items,{id=item.id,type=enum('MarketplaceProductType',item.type),status=status})
        if statuses[item.id]==enum('MarketplaceItemPurchaseStatus','Success')then out.RobuxSpent+=item.priceRobux end
    end
    return out
end
function module.dismiss_status(view)
    return enum('MarketplaceBulkPurchasePromptStatus',view=='Request'and'Aborted'or view=='Error'and'Error'or'Completed')
end
return module
"#;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_bulk_response_mapping_and_cancellation() -> mlua::Result<()> {
        let lua=mlua::Lua::new();
        let ctx=lua.create_table()?;
        ctx.set("enum",lua.create_function(|_,(kind,name):(String,String)|Ok(format!("{kind}.{name}")))?)?;
        lua.globals().set("bulk",lua.load(SOURCE).call::<mlua::Table>(ctx)?)?;
        lua.load(r#"
local data={
 {id='owned',type='AvatarAsset',priceRobux=500,purchaseableState='AlreadyOwned'},
 {id='a',type='AvatarAsset',priceRobux=10,purchaseableState='Purchaseable'},
 {id='b',type='AvatarBundle',priceRobux=20,purchaseableState='Purchaseable'},
 {id='unavailable',type='AvatarAsset',priceRobux=900,purchaseableState='NotForSale'},
}
local items=bulk.normalise(data)
assert(items[2].purchasableState=='Purchasable'and data[2].purchasableState==nil)
local cancelled=bulk.results(items,{})
assert(cancelled.RobuxSpent==0 and #cancelled.Items==4)
for _,item in cancelled.Items do assert(item.status=='MarketplaceItemPurchaseStatus.SystemError')end
assert(bulk.dismiss_status('Request')=='MarketplaceBulkPurchasePromptStatus.Aborted')
local response={fulfillmentGroups={{lineItems={
 {status='SUCCEEDED',errorReason='NOT_FOR_SALE'},
 {status='FAILED',errorReason='ALREADY_OWNED'},
}}, {lineItems={{status='SUCCEEDED'}}}}}
local statuses=bulk.statuses(response,items)
assert(statuses.a=='MarketplaceItemPurchaseStatus.Success')
assert(statuses.b=='MarketplaceItemPurchaseStatus.AlreadyOwned')
assert(statuses.owned==nil and statuses.unavailable==nil)
local result=bulk.results(items,statuses)
assert(result.RobuxSpent==10 and #result.Items==4)
assert(result.Items[1].id=='owned'and result.Items[3].type=='MarketplaceProductType.AvatarBundle')
assert(bulk.view(statuses)=='PartialSuccess')
assert(bulk.dismiss_status('PartialSuccess')=='MarketplaceBulkPurchasePromptStatus.Completed')
local successes=bulk.statuses({fulfillmentGroups={{lineItems={{status='SUCCEEDED'},{status='SUCCEEDED'}}}}},items)
assert(bulk.view(successes)=='Success'and bulk.results(items,successes).RobuxSpent==30)
assert(bulk.dismiss_status('Success')=='MarketplaceBulkPurchasePromptStatus.Completed')
assert(bulk.view({a='MarketplaceItemPurchaseStatus.SoldOut',b='MarketplaceItemPurchaseStatus.SoldOut'})=='Error')
assert(bulk.dismiss_status('Error')=='MarketplaceBulkPurchasePromptStatus.Error')
-- Current missing-fulfillment-groups handling yields an empty map, which has
-- no common status and therefore reaches PartialSuccess (not fake Success).
assert(next(bulk.statuses({},items))==nil and bulk.view({})=='PartialSuccess')
local truncated=bulk.statuses({fulfillmentGroups={{lineItems={{status='SUCCEEDED'}}}}},items)
assert(truncated.b==nil and bulk.results(items,truncated).Items[3].status=='MarketplaceItemPurchaseStatus.SystemError')
        "#).exec()
    }
    #[test]
    fn native_bulk_error_reason_matrix() -> mlua::Result<()> {
        let lua=mlua::Lua::new();let ctx=lua.create_table()?;
        ctx.set("enum",lua.create_function(|_,(_kind,name):(String,String)|Ok(name))?)?;
        lua.globals().set("bulk",lua.load(SOURCE).call::<mlua::Table>(ctx)?)?;
        lua.load(r#"
local expected={INSUFFICIENT_ROBUX='InsufficientRobux',QUOTA_EXCEEDED='QuotaExceeded',NOT_FOR_SALE='NotForSale',NOT_AVAILABLE_FOR_PURCHASER='NotAvailableForPurchaser',PRICE_MISMATCH='PriceMismatch',SOLD_OUT='SoldOut',PURCHASER_IS_SELLER='PurchaserIsSeller',QUANTITY_LIMIT_EXCEEDED='QuantityLimitExceeded',ALREADY_OWNED='AlreadyOwned',INSUFFICIENT_MEMBERSHIP='InsufficientMembership',PURCHASE_PLACE_INVALID='PlaceInvalid'}
for reason,status in expected do assert(bulk.line_status({status='FAILED',errorReason=reason})==status)end
assert(bulk.line_status({status='UNKNOWN',errorReason='NEW_REASON'})=='SystemError')
assert(bulk.line_status({})=='SystemError')
        "#).exec()
    }
}
